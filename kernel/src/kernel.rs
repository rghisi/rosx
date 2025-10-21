use alloc::boxed::Box;
use core::ptr::{null, null_mut};
use cpu::{Cpu};
use kconfig::KConfig;
use kprintln;
use state::{ExecutionState};
use task::{SharedTask, Task};
use task_scheduler::TaskScheduler;

static mut SELF: *mut Kernel = null_mut();

//This will not work for SMP, need to remove
static mut SCHEDULER_TASK_PTR: Option<*mut dyn TaskScheduler> = None;

pub struct Kernel {
    kconfig: &'static KConfig,
    cpu: &'static dyn Cpu,
    scheduler: Box<dyn TaskScheduler>,
    pub(crate) execution_state: ExecutionState
}

impl Kernel {

    pub fn new(
        kconfig: &'static KConfig
    ) -> Self {
        let cpu = kconfig.cpu;
        let scheduler = (kconfig.scheduler)();

        let mut scheduler_task = Task::new(
            0,
            "[K] Scheduler Thread",
            task_wrapper as usize,
            scheduler_wrapper as usize
        );
        cpu.initialize_task(scheduler_task.as_mut());

        Kernel {
            kconfig,
            cpu,
            scheduler,
            execution_state: ExecutionState {
                scheduler_task,
                current_task: None,
                cpu
            }
        }
    }

    pub fn setup(&mut self) {
        unsafe {
            SELF = self;
            SCHEDULER_TASK_PTR = Some(self.scheduler.as_mut());
        }
        let mut idle_task = (self.kconfig.idle_task)();
        self.cpu.initialize_task(idle_task.as_mut());
        let _ = self.scheduler.set_idle_task(idle_task);
        self.cpu.enable_interrupts();
    }

    pub fn schedule(&mut self, mut task: SharedTask) {
        self.cpu.initialize_task(task.as_mut());
        let _ = self.scheduler.push_task(task);
    }

    pub fn start(&mut self) {
        let scheduler_thread_stack_pointer = self.execution_state.scheduler_task.as_ref().stack_pointer();
        self.cpu.swap_context(null_mut(), scheduler_thread_stack_pointer);
    }
}

#[inline(always)]
pub fn task_yield() {
    unsafe {
        (*SELF).execution_state.switch_to_scheduler();
    }
}

#[inline(always)]
pub fn switch_to_task(task: SharedTask) -> SharedTask {
    unsafe {
        (*SELF).execution_state.switch_to_task(task)
    }
}

#[inline(always)]
pub(crate) fn terminate_current_task() {
    unsafe {
        if let Some(mut task) = (*SELF).execution_state.current_task.take() {
            task.set_terminated();
            (*SELF).execution_state.current_task = Some(task);
        }
    }
}

pub(crate) extern "C" fn task_wrapper(actual_entry: usize) {
    let task_fn: fn() = unsafe { core::mem::transmute(actual_entry) };
    task_fn();

    terminate_current_task();
    task_yield();
}

extern "C" fn scheduler_wrapper() -> ! {
    unsafe {
        if let Some(ptr) = SCHEDULER_TASK_PTR {
            let main_thread = &mut *ptr;
            main_thread.run();
        }
    }

    loop {}
}