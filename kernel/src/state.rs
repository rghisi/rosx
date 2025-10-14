use core::ptr::null_mut;
use cpu::Cpu;
use main_thread::MainThread;
use task::{SharedTask, Task};

struct ExecutionContext {
    cpu: &'static dyn Cpu,
    current_task: Option<SharedTask>,
    scheduler_task: Option<SharedTask>
}

impl ExecutionContext {
    fn switch_to_task(&mut self, task: SharedTask) -> Option<SharedTask> {
        let task_stack_pointer = task.stack_pointer();
        self.current_task = Some(task);
        let mut main_thread_task = self.scheduler_task.take().expect("no main thread task");
        let main_thread_stack_pointer = main_thread_task.stack_pointer_mut();
        self.scheduler_task = Some(main_thread_task);
        self.cpu.swap_context(main_thread_stack_pointer, task_stack_pointer);
        self.current_task.take()
    }

    fn switch_to_scheduler(&mut self) {
        let task_stack_pointer_reference = if let Some(mut task) = self.current_task.take() {
             task.stack_pointer_mut()
        } else {
            null_mut()
        };

        CURRENT_TASK = Some(task);
        cpu.swap_context(task_stack_pointer_reference, main.as_mut().unwrap().stack_pointer())
    }
}

pub(crate) static mut MAIN_THREAD_PTR: Option<*mut MainThread> = None;
pub(crate) static mut MAIN_THREAD_TASK: Option<SharedTask> = None;
pub(crate) static mut MAIN_THREAD_TASK_PTR: Option<*mut Task> = None;
pub(crate) static mut CURRENT_TASK: Option<SharedTask> = None;
pub(crate) static mut CPU_PTR: Option<&'static dyn Cpu> = None;