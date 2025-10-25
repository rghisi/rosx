use cpu::Cpu;
use task::{SharedTask};

pub struct ExecutionState {
    pub(crate) main_thread: SharedTask,
    pub(crate) current_task: Option<SharedTask>,
    pub(crate) cpu: &'static dyn Cpu
}

impl ExecutionState {
    #[inline(always)]
    pub(crate) fn switch_to_task(&mut self, task: SharedTask) -> SharedTask {
        let task_stack_pointer = task.stack_pointer();
        self.current_task = Some(task);
        let scheduler_stack_pointer_pointer = self.main_thread.as_mut().stack_pointer_mut();
        self.cpu.swap_context(scheduler_stack_pointer_pointer, task_stack_pointer);

        self.current_task.take().unwrap()
    }

    #[inline(always)]
    pub(crate) fn switch_to_scheduler(&mut self) {
        if let Some(mut task) = self.current_task.take() {
            let task_stack_pointer_reference = task.stack_pointer_mut();
            self.current_task = Some(task);
            let scheduler_stack_pointer = self.main_thread.as_ref().stack_pointer();
            self.cpu.swap_context(task_stack_pointer_reference, scheduler_stack_pointer)
        }
    }
}
