use crate::cpu::Cpu;
use crate::kernel::TASK_MANAGER;
use crate::task::TaskHandle;
use crate::task::TaskState::Blocked;

pub struct ExecutionState {
    pub(crate) main_thread: TaskHandle,
    pub(crate) current_task: Option<TaskHandle>,
    pub(crate) preemption_enabled: bool,
    pub(crate) cpu: &'static dyn Cpu,
}

impl ExecutionState {
    #[inline(always)]
    pub(crate) fn switch_to_task(&mut self, task_handle: TaskHandle) -> TaskHandle {
        let task_stack_pointer = TASK_MANAGER
            .lock()
            .borrow()
            .get_task_stack_pointer(task_handle);
        self.current_task = Some(task_handle);
        let scheduler_stack_pointer_pointer = TASK_MANAGER
            .lock()
            .borrow_mut()
            .get_task_stack_pointer_ref(self.main_thread);
        self.cpu
            .swap_context(scheduler_stack_pointer_pointer, task_stack_pointer);

        self.current_task.take().unwrap()
    }

    #[inline(always)]
    pub(crate) fn switch_to_scheduler(&mut self) {
        if let Some(task_handle) = self.current_task.take() {
            let task_stack_pointer_reference = TASK_MANAGER
                .lock()
                .borrow_mut()
                .get_task_stack_pointer_ref(task_handle);
            self.current_task = Some(task_handle);
            let scheduler_stack_pointer = TASK_MANAGER
                .lock()
                .borrow()
                .get_task_stack_pointer(self.main_thread);
            self.cpu
                .swap_context(task_stack_pointer_reference, scheduler_stack_pointer)
        }
    }

    pub(crate) fn block_current_task(&mut self) {
        if let Some(task_handle) = self.current_task {
            TASK_MANAGER
                .lock()
                .borrow_mut()
                .set_state(task_handle, Blocked);
        }
    }

    pub(crate) fn current_task(&self) -> TaskHandle {
        self.current_task.unwrap()
    }
}
