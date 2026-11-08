use crate::platform::cpu::Cpu;
#[cfg(not(test))]
use crate::platform::output::{KernelOutput, setup_default_output};
use crate::platform::elf::ElfArch;
use crate::future::{TaskCompletionFuture, TimeFuture};
use crate::future::waitset::WaitsetTake;
use system::waitset::WaitEvent;
use crate::kconfig::KConfig;
use crate::kernel_services::services;
#[cfg(not(test))]
use crate::kprintln;
#[cfg(not(test))]
use crate::platform::portio::{PortIo, PortRange};

use crate::state::{ExecutionContext, ExecutionState};
use crate::task::TaskState;
use crate::task::TaskState::Terminated;
use crate::task::{SharedTask, Task, TaskHandle, YieldReason};
use crate::ports::driven::ForExpiringTimers;
use crate::ports::driven::ForManagingTasks;
use crate::ports::driven::ForReadingSystemTime;
use crate::ports::driven::ForSwitchingTaskContext;
use crate::ports::driven::SwitchOutcome;
use alloc::boxed::Box;
use core::ptr::null_mut;
use crate::collections::arena::Error;
use crate::future::Future;
use crate::handle::FutureHandle;
#[cfg(not(test))]
use crate::memory::memory_manager::MEMORY_MANAGER;
#[cfg(not(test))]
use crate::memory::MemoryBlocks;
use crate::kernel_cell::KernelCell;

static KERNEL_PTR: KernelCell<*mut Kernel> = KernelCell::new(null_mut());

struct KernelContextSwitcher;
static KERNEL_CONTEXT_SWITCHER: KernelContextSwitcher = KernelContextSwitcher;

impl ForSwitchingTaskContext for KernelContextSwitcher {
    fn switch_to_task(&self, handle: TaskHandle) -> SwitchOutcome {
        let returned = kernel().execution_state.switch_to_task(handle);
        services()
            .task_manager
            .borrow_mut()
            .observe_task_stack(returned);
        match services().task_manager.borrow().get_state(returned) {
            TaskState::Running => {
                let reason = services().task_manager.borrow().get_yield_reason(returned).unwrap_or(YieldReason::Voluntary);
                SwitchOutcome::Yielded(returned, reason)
            }
            TaskState::Blocked => SwitchOutcome::Blocked(returned),
            TaskState::Terminated => SwitchOutcome::Terminated(returned),
            _ => SwitchOutcome::Unchanged(returned),
        }
    }
}

pub(crate) struct KernelTimeSource;
pub(crate) static KERNEL_TIME_SOURCE: KernelTimeSource = KernelTimeSource;

impl ForReadingSystemTime for KernelTimeSource {
    fn now(&self) -> u64 {
        kernel().get_system_time()
    }
}

pub(crate) struct KernelTimerExpiry;
pub(crate) static KERNEL_TIMER_EXPIRY: KernelTimerExpiry = KernelTimerExpiry;

impl ForExpiringTimers for KernelTimerExpiry {
    fn pop_expired(&self, now: u64) -> Option<alloc::vec::Vec<FutureHandle>> {
        services().timer_manager.borrow_mut().pop_expired(now)
    }
}

pub(crate) struct KernelTaskManager;
pub(crate) static KERNEL_TASK_MANAGER: KernelTaskManager = KernelTaskManager;

impl ForManagingTasks for KernelTaskManager {
    fn get_state(&self, handle: TaskHandle) -> TaskState {
        services().task_manager.borrow().get_state(handle)
    }
    fn set_state(&self, handle: TaskHandle, state: TaskState) {
        services().task_manager.borrow_mut().set_state(handle, state)
    }
    fn remove_task(&self, handle: TaskHandle) {
        services().task_manager.borrow_mut().remove_task(handle)
    }
}

pub fn kernel() -> &'static mut Kernel {
    unsafe { &mut **KERNEL_PTR.borrow() }
}

pub struct Kernel {
    cpu: &'static dyn Cpu,
    pub(crate) elf_arch: &'static dyn ElfArch,
    #[cfg(not(test))]
    pub(crate) port_io: &'static dyn PortIo,
    #[cfg(not(test))]
    pub(crate) port_pool: &'static [PortRange],
    pub(crate) execution_state: ExecutionState,
}

impl Kernel {
    pub fn new(kconfig: &'static KConfig) -> Self {
        let cpu = kconfig.cpu;
        let elf_arch = kconfig.elf_arch;
        #[cfg(not(test))]
        let port_io = kconfig.port_io;
        #[cfg(not(test))]
        let port_pool = kconfig.port_pool;
        crate::kernel_services::init();
        crate::irq_guard::init_cpu(cpu);
        let scheduler_task = Task::new("[K] Main Thread", main_thread_run as *const () as usize, 0);
        let scheduler_task_handler = services()
            .task_manager
            .borrow_mut()
            .add_task(scheduler_task)
            .unwrap();
        services()
            .task_manager
            .borrow_mut()
            .borrow_task_mut(scheduler_task_handler)
            .unwrap()
            .prepare_entry(cpu);

        let scheduler = (kconfig.scheduler_factory)(&KERNEL_CONTEXT_SWITCHER, services().timer_handler);
        services().scheduler.replace(*scheduler);

        Kernel {
            cpu,
            elf_arch,
            #[cfg(not(test))]
            port_io,
            #[cfg(not(test))]
            port_pool,
            execution_state: ExecutionState {
                scheduler: scheduler_task_handler,
                current_task: None,
                preemption_enabled: false,
                need_resched: false,
                execution_context: ExecutionContext::Kernel,
                cpu,
            },
        }
    }

    pub fn setup(&mut self) {
        *KERNEL_PTR.borrow_mut() = self;
        #[cfg(not(test))]
        services().memory_manager.setup(self.cpu);
        self.cpu.setup();
        let idle_task = crate::task::idle_task_factory(self.cpu);
        let task_handle = services()
            .task_manager
            .borrow_mut()
            .add_task(idle_task)
            .unwrap();
        services()
            .task_manager
            .borrow_mut()
            .borrow_task_mut(task_handle)
            .unwrap()
            .prepare_entry(self.cpu);
        let _ = services().scheduler.borrow_mut().set_idle_task(task_handle);
    }

    pub fn start(&mut self) {
        let main_thread_handle = self.execution_state.scheduler;
        let scheduler_thread_stack_pointer = services()
            .task_manager
            .borrow()
            .get_task_stack_pointer(main_thread_handle);
        self.cpu.enable_interrupts();
        self.execution_state.preemption_enabled = false;
        let state_pointer = &mut self.execution_state as *mut ExecutionState as *mut usize;
        self.cpu
            .swap_context(state_pointer, null_mut(), scheduler_thread_stack_pointer);
    }

    pub fn schedule(&mut self, task: SharedTask) -> Result<FutureHandle, ()> {
        self.schedule_with_task_handle(task).map(|(_task_handle, future_handle)| future_handle)
    }

    pub(crate) fn schedule_with_task_handle(&mut self, task: SharedTask) -> Result<(TaskHandle, FutureHandle), ()> {
        let prev = self.execution_state.preemption_enabled;
        self.execution_state.preemption_enabled = false;
        let result = services().task_manager.borrow_mut().add_task(task);
        let paired = match result {
            Ok(task_handle) => {
                let future = Box::new(TaskCompletionFuture::new(task_handle));
                let future_handle = services().future_registry.borrow_mut().register(future);
                if let Some(fh) = future_handle {
                    services().task_manager.borrow_mut().set_completion_future(task_handle, fh);
                }
                self.schedule_task(task_handle);
                future_handle.map(|fh| (task_handle, fh))
            }
            Err(_) => {
                panic!("Not able to create new task");
            }
        };
        self.execution_state.preemption_enabled = prev;
        paired.ok_or(())
    }

    pub fn sleep(&mut self, millis: u64) {
        let future = Box::new(TimeFuture::new());
        let handle = services().future_registry
            .borrow_mut()
            .register(future)
            .expect("Failed to register sleep future");
        services().timer_manager.borrow_mut().add_sleep(
            kernel().get_system_time(),
            millis,
            handle,
        );
        let _ = self.wait_future(handle);
    }

    pub fn wait_future(&mut self, handle: FutureHandle) -> Result<Box<dyn Future + Send + Sync>, Error> {
        if !self.is_future_completed(handle) {
            let task_handle = self.execution_state.current_task();
            self.execution_state.block_current_task();
            services().future_registry.borrow_mut().register_waiter(handle, task_handle);
            self.execution_state.switch_to_scheduler();
        }
        services().future_registry.borrow_mut().consume(handle)
    }

    pub fn wait<T: Future + Send + Sync + 'static>(&mut self, handle: FutureHandle) -> Option<Box<T>> {
        self.wait_future(handle).ok()?.into_any().downcast::<T>().ok()
    }

    pub fn wait_waitset(&mut self, handle: FutureHandle) -> Result<WaitEvent, Error> {
        loop {
            match services().future_registry.borrow_mut().take_pending(handle) {
                WaitsetTake::Event(event) => return Ok(event),
                WaitsetTake::Empty => {
                    self.execution_state.block_current_task();
                    let task_handle = self.execution_state.current_task();
                    services().future_registry.borrow_mut().register_waiter(handle, task_handle);
                    self.execution_state.switch_to_scheduler();
                }
                WaitsetTake::Gone => return Err(Error::NotFound),
            }
        }
    }

    pub fn is_future_completed(&self, handle: FutureHandle) -> bool {
        services().future_registry.borrow_mut().get(handle).unwrap_or(true)
    }

    pub fn task_yield(&mut self) {
        if let Some(task_handle) = self.execution_state.current_task {
            services().task_manager.borrow_mut().set_yield_reason(task_handle, YieldReason::Voluntary);
        }
        self.execution_state.switch_to_scheduler();
    }

    pub fn mark_resched() {
        if !KERNEL_PTR.borrow().is_null() {
            kernel().execution_state.need_resched = true;
        }
    }

    pub fn maybe_resched(&mut self) {
        if !self.execution_state.need_resched {
            return;
        }
        if !self.execution_state.preemption_enabled {
            return;
        }
        self.execution_state.need_resched = false;
        if services().scheduler.borrow_mut().should_preempt() {
            if let Some(task_handle) = self.execution_state.current_task {
                services().task_manager.borrow_mut().set_yield_reason(task_handle, YieldReason::Preempted);
            }
            self.execution_state.switch_to_scheduler();
        }
    }

    pub fn switch_to_task(&mut self, task_handle: TaskHandle) -> TaskHandle {
        self.execution_state.switch_to_task(task_handle)
    }

    pub(crate) fn terminate_and_yield(&mut self) -> ! {
        self.terminate_with_status(system::exit::encode(system::exit::EXIT_REASON_CLEAN, 0))
    }

    pub(crate) fn terminate_with_status(&mut self, status: i32) -> ! {
        self.execution_state.preemption_enabled = false;
        if let Some(task_handle) = self.execution_state.current_task.take() {
            let group = services().task_manager.borrow().get_group(task_handle);
            services()
                .task_manager
                .borrow_mut()
                .set_state(task_handle, Terminated);
            crate::platform::portio::revoke(task_handle);
            if let Some(completion_handle) =
                services().task_manager.borrow().get_completion_future(task_handle) {
                    services().future_registry.borrow_mut().notify_with_status(completion_handle, status);
                }
            if let Some(group) = group {
                if !services().task_manager.borrow().group_has_live_members(group) {
                    unsafe { services().memory_manager.release_group_allocations(group.value()) };
                }
            }
            self.execution_state.current_task = Some(task_handle);
        }
        self.execution_state.switch_to_scheduler();
        unreachable!()
    }

    #[inline(always)]
    pub fn get_system_time(&self) -> u64 {
        self.cpu.get_system_time()
    }

    pub fn interrupts_enabled(&self) -> bool {
        self.cpu.are_interrupts_enabled()
    }

    fn schedule_task(&mut self, task_handle: TaskHandle) {
        {
            let result = services().task_manager.borrow_mut().borrow_task_mut(task_handle);
            match result {
                Ok(task) => {
                    task.prepare_entry(self.cpu);
                }
                Err(_) => {
                    panic!("Not able to schedule task");
                }
            }
        }
        services().scheduler.borrow_mut().push_task(task_handle);
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubCpu;

    impl Cpu for StubCpu {
        fn setup(&self) {}
        fn enable_interrupts(&self) {}
        fn disable_interrupts(&self) {}
        fn are_interrupts_enabled(&self) -> bool {
            false
        }
        fn initialize_stack(&self, stack_pointer: usize, _entry_point: usize, _param1: usize, _param2: usize) -> usize {
            stack_pointer
        }
        fn swap_context(&self, _state: *mut usize, _stack_pointer_to_store: *mut usize, _stack_pointer_to_load: usize) {}
        fn get_system_time(&self) -> u64 {
            0
        }
        fn halt(&self) {}
    }

    struct StubElfArch;

    impl ElfArch for StubElfArch {
        fn apply_relocation(&self, _base: usize, _offset: usize, _info: u64, _addend: i64) {}
    }

    fn add_created_task(name: &'static str) -> TaskHandle {
        services().task_manager.borrow_mut().add_task(Task::new(name, 0x1000, 0)).unwrap()
    }

    fn install_stub_kernel(scheduler: TaskHandle, current_task: Option<TaskHandle>, preemption_enabled: bool, need_resched: bool) {
        let cpu = Box::leak(Box::new(StubCpu)) as &'static dyn Cpu;
        let kernel: &'static mut Kernel = Box::leak(Box::new(Kernel {
            cpu,
            elf_arch: Box::leak(Box::new(StubElfArch)) as &'static dyn ElfArch,
            execution_state: ExecutionState {
                scheduler,
                current_task,
                preemption_enabled,
                need_resched,
                execution_context: ExecutionContext::UserTask,
                cpu,
            },
        }));
        *KERNEL_PTR.borrow_mut() = kernel as *mut Kernel;
    }

    fn yield_reason_of(handle: TaskHandle) -> Option<YieldReason> {
        services().task_manager.borrow().get_yield_reason(handle)
    }

    #[test]
    fn consult_without_intent_is_inert() {
        crate::kernel_services::init();
        let scheduler = add_created_task("[T] consult-scheduler");
        let current = add_created_task("[T] consult-current");
        install_stub_kernel(scheduler, Some(current), true, false);
        kernel().maybe_resched();
        assert!(!kernel().execution_state.need_resched);
        assert_eq!(yield_reason_of(current), None);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn consult_with_intent_and_enabled_preemption_clears_intent_and_stamps_preemption() {
        crate::kernel_services::init();
        let scheduler = add_created_task("[T] fire-scheduler");
        let current = add_created_task("[T] fire-current");
        install_stub_kernel(scheduler, Some(current), true, true);
        kernel().maybe_resched();
        assert!(!kernel().execution_state.need_resched);
        assert_eq!(yield_reason_of(current), Some(YieldReason::Preempted));
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn intent_survives_disabled_window_until_next_consult() {
        crate::kernel_services::init();
        let scheduler = add_created_task("[T] defer-scheduler");
        let current = add_created_task("[T] defer-current");
        install_stub_kernel(scheduler, Some(current), false, true);
        kernel().maybe_resched();
        assert!(kernel().execution_state.need_resched);
        assert_eq!(yield_reason_of(current), None);
        kernel().execution_state.preemption_enabled = true;
        kernel().maybe_resched();
        assert!(!kernel().execution_state.need_resched);
        assert_eq!(yield_reason_of(current), Some(YieldReason::Preempted));
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn mark_resched_sets_the_intent_flag() {
        crate::kernel_services::init();
        let scheduler = add_created_task("[T] mark-scheduler");
        install_stub_kernel(scheduler, None, false, false);
        Kernel::mark_resched();
        assert!(kernel().execution_state.need_resched);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn mark_resched_is_inert_before_the_kernel_exists() {
        crate::kernel_services::init();
        *KERNEL_PTR.borrow_mut() = null_mut();
        Kernel::mark_resched();
    }

    #[test]
    fn future_notify_wake_marks_resched_intent() {
        crate::kernel_services::init();
        let scheduler = add_created_task("[T] wake-scheduler");
        let sleeper = add_created_task("[T] wake-sleeper");
        install_stub_kernel(scheduler, Some(sleeper), true, false);
        let future = Box::new(TaskCompletionFuture::new(sleeper));
        let future_handle = services().future_registry.borrow_mut().register(future).expect("register");
        services().future_registry.borrow_mut().register_waiter(future_handle, sleeper);
        services().future_registry.borrow_mut().notify(future_handle);
        assert!(kernel().execution_state.need_resched);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }
}

#[cfg(not(test))]
pub fn bootstrap(
    memory_blocks: &MemoryBlocks,
    default_output: &'static dyn KernelOutput,
) {
    setup_default_output(default_output);
    MEMORY_MANAGER.bootstrap(memory_blocks);
    MEMORY_MANAGER.print_config();
    kprintln!("[KERNEL] Bootstrapped");
}

extern "C" fn main_thread_run() -> ! {
    crate::scheduler::Scheduler::run();

    panic!("Kernel main thread returned");
}
