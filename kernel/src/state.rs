use crate::cpu::Cpu;
use crate::kernel_services::services;
use crate::task::TaskHandle;
use crate::task::TaskState::Blocked;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ExecutionContext {
    Kernel,
    UserTask,
}

pub struct ExecutionState {
    pub(crate) main_thread: TaskHandle,
    pub(crate) current_task: Option<TaskHandle>,
    pub(crate) preemption_enabled: bool,
    pub(crate) execution_context: ExecutionContext,
    pub(crate) cpu: &'static dyn Cpu,
}

impl ExecutionState {
    #[inline(always)]
    pub(crate) fn switch_to_task(&mut self, task_handle: TaskHandle) -> TaskHandle {
        let task_stack_pointer = services().task_manager
            .borrow()
            .get_task_stack_pointer(task_handle);
        self.current_task = Some(task_handle);
        let scheduler_stack_pointer_pointer = services().task_manager
            .borrow_mut()
            .get_task_stack_pointer_ref(self.main_thread);

        // We are in the main thread, preemption_enabled is already false.
        self.execution_context = ExecutionContext::UserTask;
        self.cpu
            .swap_context(scheduler_stack_pointer_pointer, task_stack_pointer);
        // When we return here, we are back in the main thread.
        // preemption_enabled must remain false.
        self.execution_context = ExecutionContext::Kernel;

        self.current_task.take().unwrap()
    }

    #[inline(always)]
    pub(crate) fn switch_to_scheduler(&mut self) {
        if let Some(task_handle) = self.current_task.take() {
            let task_stack_pointer_reference = services().task_manager
                .borrow_mut()
                .get_task_stack_pointer_ref(task_handle);

            // We are leaving the task, disable preemption
            self.preemption_enabled = false;
            self.execution_context = ExecutionContext::Kernel;
            self.current_task = Some(task_handle);
            let scheduler_stack_pointer = services().task_manager
                .borrow()
                .get_task_stack_pointer(self.main_thread);

            self.cpu
                .swap_context(task_stack_pointer_reference, scheduler_stack_pointer);
            
            // When the task resumes here, re-enable preemption
            self.preemption_enabled = true;
            self.execution_context = ExecutionContext::UserTask;
        }
    }

    pub(crate) fn block_current_task(&mut self) {
        if let Some(task_handle) = self.current_task {
            services().task_manager
                .borrow_mut()
                .set_state(task_handle, Blocked);
        }
    }

    pub(crate) fn current_task(&self) -> TaskHandle {
        self.current_task.unwrap()
    }

    pub fn execution_context(&self) -> ExecutionContext {
        self.execution_context
    }
}
