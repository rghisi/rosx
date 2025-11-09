use system::message::Message;
use crate::task::{Task, TaskHandle};

pub trait Cpu {
    fn setup(&self);
    fn enable_interrupts(&self);
    fn disable_interrupts(&self);
    fn initialize_stack(&self, stack_pointer: usize, entry_point: usize, param1: usize, param2: usize) -> usize;
    fn swap_context(&self, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize);
    fn syscall(&self, message: &Message) -> usize;
    fn get_system_time(&self) -> u64;

    fn initialize_task(&self, task_handle: TaskHandle, task: &mut Task) {
        let original_sp = task.stack_pointer();

        let new_stack_pointer = self.initialize_stack(
            original_sp,
            task.entry_point(),
            task_handle.index as usize,
            task_handle.generation as usize,
        );

        task.set_stack_pointer(new_stack_pointer);
        task.set_ready();
    }
}
