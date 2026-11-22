use crate::platform::cpu::Cpu;
#[cfg(not(test))]
use crate::platform::output::{KernelOutput, setup_default_output};
use crate::platform::elf::ElfArch;
use crate::future::{FutureOwner, TaskCompletionFuture, TimeFuture};
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
use alloc::vec::Vec;
use core::ptr::null_mut;
use crate::collections::arena::Error;
use crate::elf::ElfError;
use crate::future::Future;
use crate::handle::FutureHandle;
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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum KillError {
    TargetTerminated,
    TargetRunning,
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

    pub fn schedule(
        &mut self,
        task: SharedTask,
        owner: FutureOwner,
    ) -> Result<FutureHandle, ()> {
        self.schedule_with_task_handle(task, owner).map(|(_task_handle, future_handle)| future_handle)
    }

    pub(crate) fn schedule_with_task_handle(
        &mut self,
        task: SharedTask,
        owner: FutureOwner,
    ) -> Result<(TaskHandle, FutureHandle), ()> {
        let prev = self.execution_state.preemption_enabled;
        self.execution_state.preemption_enabled = false;
        let result = services().task_manager.borrow_mut().add_task(task);
        let paired = match result {
            Ok(task_handle) => {
                let future = Box::new(TaskCompletionFuture::owned_by(task_handle));
                let future_handle = services().future_registry.borrow_mut().register(future, owner);
                match future_handle {
                    Some(future_handle) => {
                        services().task_manager.borrow_mut().set_completion_future(task_handle, future_handle);
                        self.schedule_task(task_handle);
                        Ok((task_handle, future_handle))
                    }
                    None => {
                        services().task_manager.borrow_mut().remove_task(task_handle);
                        Err(())
                    }
                }
            }
            Err(_) => Err(()),
        };
        self.execution_state.preemption_enabled = prev;
        paired
    }

    pub fn load_elf_program(
        &mut self,
        name: &str,
        elf: &[u8],
        arguments_blob: Vec<u8>,
        owner: FutureOwner,
    ) -> Result<FutureHandle, ElfError> {
        let task = crate::task::new_elf_task_with_arguments(name, elf, arguments_blob, self.elf_arch)?;
        let group = task.group();
        match self.schedule_with_task_handle(task, owner) {
            Ok((_task_handle, future_handle)) => Ok(future_handle),
            Err(()) => {
                unsafe { MEMORY_MANAGER.release_group_allocations(group.value()) };
                Err(ElfError::CannotBeScheduled)
            }
        }
    }

    pub fn sleep(&mut self, millis: u64, owner: FutureOwner) -> Result<(), ()> {
        let sleeper = self.execution_state.current_task();
        let future = Box::new(TimeFuture::owned_by(sleeper));
        let Some(handle) = services().future_registry.borrow_mut().register(future, owner) else {
            return Err(());
        };
        services().timer_manager.borrow_mut().add_sleep(
            kernel().get_system_time(),
            millis,
            handle,
        );
        let _ = self.wait_future(handle);
        Ok(())
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
            let completion = services()
                .task_manager
                .borrow()
                .get_completion_future(task_handle);
            if let Some(completion_handle) = completion {
                services().future_registry.borrow_mut().notify_with_status(completion_handle, status);
            }
            services()
                .task_manager
                .borrow_mut()
                .set_state(task_handle, Terminated);
            self.reap_task_resources(task_handle, completion);
            crate::platform::portio::revoke(task_handle);
            if let Some(group) = group {
                if !services().task_manager.borrow().group_has_live_members(group) {
                    services().future_registry.borrow_mut().reclaim_group(group);
                    unsafe { services().memory_manager.release_group_allocations(group.value()) };
                }
            }
            self.execution_state.current_task = Some(task_handle);
        }
        self.execution_state.switch_to_scheduler();
        unreachable!()
    }

    pub(crate) fn reap_task_resources(
        &mut self,
        task: TaskHandle,
        spare_completion: Option<FutureHandle>,
    ) {
        services().future_registry.borrow_mut().discard_waiter(task);
        services().ipc_manager.borrow_mut().close_connections_of(task);
        let orphaned = services()
            .future_registry
            .borrow_mut()
            .discard_owned(task, spare_completion);
        if !orphaned.is_empty() {
            services().ipc_manager.borrow_mut().discard_parked_futures(&orphaned);
            services().timer_manager.borrow_mut().discard_futures(&orphaned);
        }
    }

    pub(crate) fn kill_task(&mut self, completion: FutureHandle) -> Result<(), KillError> {
        let owner = services()
            .future_registry
            .borrow_mut()
            .completion_owner(completion)
            .ok_or(KillError::TargetTerminated)?;
        if Some(owner) == self.execution_state.current_task {
            return Err(KillError::TargetRunning);
        }
        match services().task_manager.borrow().get_state(owner) {
            TaskState::Terminated => return Err(KillError::TargetTerminated),
            TaskState::Running => return Err(KillError::TargetRunning),
            _ => {}
        }
        let prev = self.execution_state.preemption_enabled;
        self.execution_state.preemption_enabled = false;
        services()
            .future_registry
            .borrow_mut()
            .notify_with_status(completion, system::exit::interrupted_status());
        services().task_manager.borrow_mut().set_state(owner, Terminated);
        self.reap_task_resources(owner, Some(completion));
        crate::platform::portio::revoke(owner);
        let group = services().task_manager.borrow().get_group(owner);
        if let Some(group) = group {
            if !services().task_manager.borrow().group_has_live_members(group) {
                services().future_registry.borrow_mut().reclaim_group(group);
                unsafe { services().memory_manager.release_group_allocations(group.value()) };
            }
        }
        services().scheduler.borrow_mut().handle_termination(owner);
        self.execution_state.preemption_enabled = prev;
        Ok(())
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
        let future = Box::new(TaskCompletionFuture::owned_by(sleeper));
        let future_handle = services().future_registry.borrow_mut().register(future, FutureOwner::Kernel).expect("register");
        services().future_registry.borrow_mut().register_waiter(future_handle, sleeper);
        services().future_registry.borrow_mut().notify(future_handle);
        assert!(kernel().execution_state.need_resched);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    fn spawn_paired(name: &'static str) -> (TaskHandle, crate::handle::FutureHandle) {
        let task = Task::new(name, 0x1000, 0);
        kernel()
            .schedule_with_task_handle(task, FutureOwner::Kernel)
            .expect("schedule a paired task")
    }

    fn install_kill_stage() -> TaskHandle {
        crate::kernel_services::init();
        crate::scheduler::wire_scheduler_for_tests();
        let scheduler = add_created_task("[T] kill-scheduler");
        let killer = add_created_task("[T] kill-killer");
        install_stub_kernel(scheduler, Some(killer), false, false);
        killer
    }

    fn completion_status(handle: crate::handle::FutureHandle) -> i32 {
        let consumed = services()
            .future_registry
            .borrow_mut()
            .consume(handle)
            .expect("completion future stays consumable until taken");
        consumed
            .as_any()
            .downcast_ref::<TaskCompletionFuture>()
            .expect("task completion future")
            .status()
    }

    #[test]
    fn kill_ready_task_stamps_interrupted_and_clears_it_away() {
        install_kill_stage();
        let (victim, completion) = spawn_paired("[T] kill-victim");

        assert_eq!(kernel().kill_task(completion), Ok(()));

        assert_eq!(
            services().future_registry.borrow_mut().get(completion),
            Some(true)
        );
        assert_eq!(completion_status(completion), system::exit::interrupted_status());
        assert_eq!(
            services().task_manager.borrow().get_state(victim),
            crate::task::TaskState::Terminated
        );
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_wakes_a_parent_parked_on_the_completion_future() {
        let killer = install_kill_stage();
        let (_victim, completion) = spawn_paired("[T] kill-waited");
        services()
            .future_registry
            .borrow_mut()
            .register_waiter(completion, killer);

        assert_eq!(kernel().kill_task(completion), Ok(()));

        assert_eq!(
            services().task_manager.borrow().get_state(killer),
            crate::task::TaskState::Ready
        );
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_blocked_task_leaves_it_unresurrectable() {
        install_kill_stage();
        let (victim, completion) = spawn_paired("[T] kill-blocked");
        let gate = services()
            .future_registry
            .borrow_mut()
            .register(Box::new(TimeFuture::owned_by(crate::task::TaskHandle::new(u16::MAX, 0))), FutureOwner::Kernel)
            .expect("register gate future");
        services()
            .future_registry
            .borrow_mut()
            .register_waiter(gate, victim);
        services()
            .task_manager
            .borrow_mut()
            .set_state(victim, crate::task::TaskState::Blocked);

        assert_eq!(kernel().kill_task(completion), Ok(()));

        services().future_registry.borrow_mut().notify(gate);
        assert_eq!(
            services().task_manager.borrow().get_state(victim),
            crate::task::TaskState::Terminated
        );
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_after_death_reports_the_target_terminated() {
        install_kill_stage();
        let (_victim, completion) = spawn_paired("[T] kill-twice");

        assert_eq!(kernel().kill_task(completion), Ok(()));
        assert_eq!(
            kernel().kill_task(completion),
            Err(KillError::TargetTerminated)
        );
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_refuses_to_terminate_the_caller_itself() {
        install_kill_stage();
        let (self_handle, self_completion) = spawn_paired("[T] kill-self");
        kernel().execution_state.current_task = Some(self_handle);

        assert_eq!(
            kernel().kill_task(self_completion),
            Err(KillError::TargetRunning)
        );
        assert_ne!(
            services().task_manager.borrow().get_state(self_handle),
            crate::task::TaskState::Terminated
        );
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_refuses_handles_that_are_not_live_completion_futures() {
        install_kill_stage();
        let foreign = services()
            .future_registry
            .borrow_mut()
            .register(Box::new(TimeFuture::owned_by(crate::task::TaskHandle::new(u16::MAX, 0))), FutureOwner::Kernel)
            .expect("register foreign future");

        assert_eq!(
            kernel().kill_task(foreign),
            Err(KillError::TargetTerminated)
        );
        assert_eq!(
            kernel().kill_task(crate::handle::FutureHandle::new(1234, 9)),
            Err(KillError::TargetTerminated)
        );
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_surrenders_the_targets_port_grants() {
        install_kill_stage();
        let pool: &'static [crate::platform::portio::PortRange] =
            Box::leak(Box::new([crate::platform::portio::PortRange::new(0x620, 0x62F)]));
        let (victim, completion) = spawn_paired("[T] kill-ports");
        crate::platform::portio::authorize_acquire(victim, 0x620, 0x62F, pool)
            .expect("victim acquires the range");

        assert_eq!(kernel().kill_task(completion), Ok(()));

        let heir = add_created_task("[T] kill-port-heir");
        assert_eq!(
            crate::platform::portio::authorize_acquire(heir, 0x620, 0x62F, pool),
            Ok(())
        );
        crate::platform::portio::authorize_release(heir, 0x620, 0x62F).ok();
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_reclaims_the_group_only_when_the_last_member_dies() {
        install_kill_stage();
        let group = crate::task::GroupId::allocate();
        let (_first, first_completion) = kernel()
            .schedule_with_task_handle(crate::task::new_entrypoint_task(0x1000, group), FutureOwner::Group(group))
            .expect("first group member");
        let (_second, second_completion) = kernel()
            .schedule_with_task_handle(crate::task::new_entrypoint_task(0x1000, group), FutureOwner::Group(group))
            .expect("second group member");
        let layout = core::alloc::Layout::from_size_align(2048, 8).unwrap();
        let owned = unsafe {
            services()
                .memory_manager
                .alloc_with_owner(layout, crate::memory::free_list_allocator::BlockOwner::Group(group.value()))
        };
        assert!(!owned.is_null());
        assert!(services().memory_manager.group_bytes(group.value()) > 0);

        assert_eq!(kernel().kill_task(first_completion), Ok(()));
        assert!(services().memory_manager.group_bytes(group.value()) > 0);

        assert_eq!(kernel().kill_task(second_completion), Ok(()));
        assert_eq!(services().memory_manager.group_bytes(group.value()), 0);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_drops_the_task_whose_stack_bytes_the_sweep_never_frees() {
        install_kill_stage();
        let (victim, completion) = spawn_paired("[T] kill-stack-drop");
        let group = services()
            .task_manager
            .borrow()
            .get_group(victim)
            .expect("the victim has a group");
        assert!(
            services().memory_manager.group_bytes(group.value()) > 0,
            "the victim's stack is accounted to its group"
        );

        assert_eq!(kernel().kill_task(completion), Ok(()));

        assert_eq!(
            services().memory_manager.group_bytes(group.value()),
            0,
            "deallocate_by_owner never matches Stack: only the dying Task's field drops free it"
        );
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn exit_drops_the_task_when_the_scheduler_removes_it() {
        install_kill_stage();
        let (victim, _completion) = spawn_paired("[T] exit-stack-drop");
        let group = services()
            .task_manager
            .borrow()
            .get_group(victim)
            .expect("the victim has a group");
        kernel().execution_state.current_task = Some(victim);

        let terminated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            kernel().terminate_with_status(system::exit::clean_status(0).unwrap())
        }));
        assert!(terminated.is_err());
        assert!(
            services().memory_manager.group_bytes(group.value()) > 0,
            "the dying task's stack outlives the group sweep: its Task is still in the arena"
        );

        services().scheduler.borrow_mut().handle_termination(victim);

        assert_eq!(
            services().memory_manager.group_bytes(group.value()),
            0,
            "handle_termination removes the Task and its field drops free the stack block"
        );
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    fn heap_used() -> usize {
        let stats = MEMORY_MANAGER.stats();
        stats.total_bytes - stats.free_bytes
    }

    #[test]
    fn a_schedule_failure_after_elf_construction_releases_the_group() {
        install_kill_stage();
        let elf = crate::elf::tests::minimal_elf64_fixture(0);
        let before = heap_used();
        let fillers = exhaust_future_registry();
        let result = kernel().load_elf_program(
            "[T] elf-no-slot",
            &elf,
            b"elf-no-slot\0".to_vec(),
            FutureOwner::Kernel,
        );
        release_future_registry(fillers);
        assert!(matches!(result, Err(ElfError::CannotBeScheduled)));
        assert_eq!(heap_used(), before, "a program that never got scheduled leaves no stamped block");
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    fn load_group_leader(name: &'static str, blob: &'static [u8]) -> (TaskHandle, FutureHandle, crate::task::GroupId) {
        let elf = crate::elf::tests::minimal_elf64_fixture(0);
        let leader_completion = kernel()
            .load_elf_program(name, &elf, blob.to_vec(), FutureOwner::Kernel)
            .expect("load the elf program");
        let leader = services()
            .future_registry
            .borrow_mut()
            .completion_owner(leader_completion)
            .expect("the completion names its task");
        let group = services()
            .task_manager
            .borrow()
            .get_group(leader)
            .expect("the leader has a group");
        assert!(services().memory_manager.group_bytes(group.value()) > 0);
        (leader, leader_completion, group)
    }

    #[test]
    fn a_loaded_program_group_survives_until_the_last_member_dies_via_kill() {
        install_kill_stage();
        let (_leader, leader_completion, group) = load_group_leader("elf-kill", b"elf-kill\0");
        let (_sibling, sibling_completion) = kernel()
            .schedule_with_task_handle(
                crate::task::new_entrypoint_task(0x1000, group),
                FutureOwner::Group(group),
            )
            .expect("spawn a sibling into the loaded group");

        assert_eq!(kernel().kill_task(leader_completion), Ok(()));
        assert!(
            services().memory_manager.group_bytes(group.value()) > 0,
            "the image must outlive the first member while a sibling can still execute it"
        );

        assert_eq!(kernel().kill_task(sibling_completion), Ok(()));
        assert_eq!(services().memory_manager.group_bytes(group.value()), 0);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn a_loaded_program_group_survives_until_the_last_member_dies_via_exit() {
        install_kill_stage();
        let (leader, _leader_completion, group) = load_group_leader("elf-exit", b"elf-exit\0");
        let (sibling, _sibling_completion) = kernel()
            .schedule_with_task_handle(
                crate::task::new_entrypoint_task(0x1000, group),
                FutureOwner::Group(group),
            )
            .expect("spawn a sibling into the loaded group");

        kernel().execution_state.current_task = Some(leader);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            kernel().terminate_with_status(system::exit::clean_status(0).unwrap())
        }));
        assert!(outcome.is_err());
        services().scheduler.borrow_mut().handle_termination(leader);
        assert!(
            services().memory_manager.group_bytes(group.value()) > 0,
            "the image must outlive the first member while a sibling can still execute it"
        );

        kernel().execution_state.current_task = Some(sibling);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            kernel().terminate_with_status(system::exit::clean_status(0).unwrap())
        }));
        assert!(outcome.is_err());
        assert!(
            services().memory_manager.group_bytes(group.value()) > 0,
            "the dying member's stack outlives the sweep until the scheduler removes it"
        );
        services().scheduler.borrow_mut().handle_termination(sibling);
        assert_eq!(services().memory_manager.group_bytes(group.value()), 0);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_discards_the_owners_other_futures_and_spares_the_completion() {
        install_kill_stage();
        let (victim, completion) = spawn_paired("[T] kill-swept");
        let sleeper = services()
            .future_registry
            .borrow_mut()
            .register(Box::new(TimeFuture::owned_by(victim)), FutureOwner::Kernel)
            .expect("register the victim's sleep future");

        assert_eq!(kernel().kill_task(completion), Ok(()));

        assert_eq!(services().future_registry.borrow_mut().get(sleeper), None);
        assert_eq!(completion_status(completion), system::exit::interrupted_status());
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_closes_the_victims_ipc_connections() {
        install_kill_stage();
        services()
            .ipc_manager
            .borrow_mut()
            .bind_service("KILL-IPC")
            .expect("bind the kill-ipc service");
        let (victim, completion) = spawn_paired("[T] kill-ipc");
        let connection = services()
            .ipc_manager
            .borrow_mut()
            .connect("KILL-IPC", victim)
            .expect("victim connects");
        let server = add_created_task("[T] kill-ipc-server");

        assert_eq!(kernel().kill_task(completion), Ok(()));

        assert_eq!(
            services().ipc_manager.borrow_mut().send_message(
                server,
                &system::ipc::Message::new(
                    connection.pack() as u32,
                    &[0u8; system::ipc::MESSAGE_PAYLOAD_BYTES]
                )
            ),
            Err(crate::ipc::errors::IpcSendError::ConnectionNotFound)
        );
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn kill_abandons_the_parked_receive_on_the_victims_connection() {
        install_kill_stage();
        services()
            .ipc_manager
            .borrow_mut()
            .bind_service("KILL-PARK")
            .expect("bind the kill-park service");
        let (victim, completion) = spawn_paired("[T] kill-park");
        let connection = services()
            .ipc_manager
            .borrow_mut()
            .connect("KILL-PARK", victim)
            .expect("victim connects");
        let parked = services()
            .ipc_manager
            .borrow_mut()
            .receive_message_async(connection, FutureOwner::Kernel, victim)
            .unwrap();
        assert_eq!(
            services().future_registry.borrow_mut().get(parked),
            Some(false)
        );

        assert_eq!(kernel().kill_task(completion), Ok(()));

        assert_eq!(services().future_registry.borrow_mut().get(parked), None);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn the_reap_clears_connections_futures_and_timer_entries_alike() {
        install_kill_stage();
        services()
            .ipc_manager
            .borrow_mut()
            .bind_service("REAP-BODY")
            .expect("bind the reap-body service");
        let (victim, completion) = spawn_paired("[T] reap-body");
        let connection = services()
            .ipc_manager
            .borrow_mut()
            .connect("REAP-BODY", victim)
            .expect("victim connects");
        let parked = services()
            .ipc_manager
            .borrow_mut()
            .receive_message_async(connection, FutureOwner::Kernel, victim)
            .unwrap();
        let sleeper = services()
            .future_registry
            .borrow_mut()
            .register(Box::new(TimeFuture::owned_by(victim)), FutureOwner::Kernel)
            .expect("register the victim's sleep future");
        services()
            .timer_manager
            .borrow_mut()
            .add_sleep(0, 10_000, sleeper);
        let server = add_created_task("[T] reap-body-server");

        services()
            .task_manager
            .borrow_mut()
            .set_state(victim, crate::task::TaskState::Terminated);
        kernel().reap_task_resources(victim, Some(completion));

        assert_eq!(services().future_registry.borrow_mut().get(parked), None);
        assert_eq!(services().future_registry.borrow_mut().get(sleeper), None);
        assert_eq!(completion_status(completion), system::exit::clean_status(0).unwrap());
        assert_eq!(
            services().ipc_manager.borrow_mut().send_message(
                server,
                &system::ipc::Message::new(
                    connection.pack() as u32,
                    &[0u8; system::ipc::MESSAGE_PAYLOAD_BYTES]
                )
            ),
            Err(crate::ipc::errors::IpcSendError::ConnectionNotFound)
        );
        assert!(!services()
            .timer_manager
            .borrow_mut()
            .pop_expired(20_000)
            .is_some_and(|expired| expired.contains(&sleeper)));
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    fn exhaust_future_registry() -> Vec<FutureHandle> {
        let mut fillers = Vec::new();
        while let Some(handle) = services()
            .future_registry
            .borrow_mut()
            .register(Box::new(TimeFuture::owned_by(crate::task::TaskHandle::new(u16::MAX, 0))), FutureOwner::Kernel)
        {
            fillers.push(handle);
        }
        fillers
    }

    fn release_future_registry(fillers: Vec<FutureHandle>) {
        for handle in fillers {
            services().future_registry.borrow_mut().consume(handle).unwrap();
        }
    }

    #[test]
    fn sleep_reports_exhaustion_instead_of_panicking() {
        crate::kernel_services::init();
        let scheduler = add_created_task("[T] exhausted-sleep-scheduler");
        let sleeper = add_created_task("[T] exhausted-sleeper");
        install_stub_kernel(scheduler, Some(sleeper), false, false);
        let fillers = exhaust_future_registry();
        assert_eq!(kernel().sleep(1, FutureOwner::Kernel), Err(()));
        release_future_registry(fillers);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn schedule_refuses_a_task_when_the_registry_is_exhausted() {
        crate::kernel_services::init();
        let scheduler = add_created_task("[T] ghost-scheduler");
        install_stub_kernel(scheduler, None, false, false);
        let fillers = exhaust_future_registry();
        let live_before = services().task_manager.borrow().snapshot_processes().len();
        let task = Task::new("[T] ghost-attempt", 0x1000, 0);
        assert_eq!(
            kernel().schedule_with_task_handle(task, FutureOwner::Kernel),
            Err(())
        );
        assert_eq!(
            services().task_manager.borrow().snapshot_processes().len(),
            live_before
        );
        release_future_registry(fillers);
        *KERNEL_PTR.borrow_mut() = null_mut();
    }

    #[test]
    fn termination_of_the_last_group_member_reclaims_the_groups_futures() {
        crate::kernel_services::init();
        let scheduler = add_created_task("[T] reclaim-hook-scheduler");
        let dying = add_created_task("[T] reclaim-hook-dying");
        install_stub_kernel(scheduler, Some(dying), false, false);
        let dying_group = services().task_manager.borrow().get_group(dying).unwrap();
        let parent_group = services().task_manager.borrow().get_group(scheduler).unwrap();

        let abandoned = services()
            .future_registry
            .borrow_mut()
            .register(Box::new(TimeFuture::owned_by(crate::task::TaskHandle::new(u16::MAX, 0))), FutureOwner::Group(dying_group))
            .unwrap();
        let report_to_parent = services()
            .future_registry
            .borrow_mut()
            .register(Box::new(TaskCompletionFuture::owned_by(dying)), FutureOwner::Group(parent_group))
            .unwrap();
        services().task_manager.borrow_mut().set_completion_future(dying, report_to_parent);

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            kernel().terminate_with_status(system::exit::clean_status(5).unwrap())
        }));
        assert!(outcome.is_err());

        assert_eq!(services().future_registry.borrow_mut().get(abandoned), None);
        let report = services().future_registry.borrow_mut().consume(report_to_parent).unwrap();
        let report = report.into_any().downcast::<TaskCompletionFuture>().unwrap();
        assert!(report.is_completed());
        assert_eq!(report.status(), system::exit::clean_status(5).unwrap());
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
