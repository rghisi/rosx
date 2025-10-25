use core::ptr::{null, null_mut};
use cpu::Cpu;
use kconfig::KConfig;
use kprintln;
use main_thread::MainThread;
use state::ExecutionState;
use task::{SharedTask, Task};
use crate::messages::HardwareInterrupt;

static mut SELF: *mut Kernel = null_mut();

//This will not work for SMP, need to remove
static mut MAIN_THREAD_PTR: Option<*mut MainThread> = None;

pub struct Kernel {
    kconfig: &'static KConfig,
    cpu: &'static dyn Cpu,
    main_thread: MainThread,
    execution_state: ExecutionState,
}

impl Kernel {

    pub fn new(
        kconfig: &'static KConfig
    ) -> Self {
        let cpu = kconfig.cpu;
        let scheduler = (kconfig.user_thread_queue)();

        let mut main_thread_task = Task::new(
            0,
            "[K] Main Thread",
            task_wrapper as usize,
            main_thread_wrapper as usize
        );
        cpu.initialize_task(main_thread_task.as_mut());

        let main_thread = MainThread::new(scheduler);

        Kernel {
            kconfig,
            cpu,
            main_thread,
            execution_state: ExecutionState {
                main_thread: main_thread_task,
                current_task: None,
                cpu
            }
        }
    }

    pub fn setup(&mut self) {
        unsafe {
            SELF = self;
            MAIN_THREAD_PTR = Some(&mut self.main_thread);
        }
        let mut idle_task = (self.kconfig.idle_task)();
        self.cpu.initialize_task(idle_task.as_mut());
        let _ = self.main_thread.set_idle_task(idle_task);
        self.cpu.enable_interrupts();
    }

    pub fn start(&mut self) {
        let scheduler_thread_stack_pointer = self.execution_state.main_thread.as_ref().stack_pointer();
        self.cpu.swap_context(null_mut(), scheduler_thread_stack_pointer);
    }

    pub fn schedule(&mut self, mut task: SharedTask) {
        self.cpu.initialize_task(task.as_mut());
        let _ = self.main_thread.push_task(task);
    }

    pub fn enqueue(&mut self, hardware_interrupt: HardwareInterrupt) {
        self.main_thread.push_hardware_interrupt(hardware_interrupt);
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
pub fn enqueue_hardware_interrupt(hardware_interrupt: HardwareInterrupt) {
    unsafe {
        (*SELF).enqueue(hardware_interrupt);
    }
}


pub(crate) extern "C" fn task_wrapper(actual_entry: usize) {
    let task_fn: fn() = unsafe { core::mem::transmute(actual_entry) };
    task_fn();

    terminate_current_task();
    task_yield();
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

extern "C" fn main_thread_wrapper() -> ! {
    unsafe {
        if let Some(ptr) = MAIN_THREAD_PTR {
            let main_thread = &mut *ptr;
            main_thread.run();
        }
    }

    loop {}
}