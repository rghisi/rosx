use crate::task::Task;

pub trait Cpu {
    fn setup(&self);
    fn enable_interrupts(&self);
    fn disable_interrupts(&self);
    fn are_interrupts_enabled(&self) -> bool;
    fn initialize_stack(
        &self,
        stack_pointer: usize,
        entry_point: usize,
        param1: usize,
        param2: usize,
    ) -> usize;
    fn swap_context(&self, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize);
    fn get_system_time(&self) -> u64;

    fn halt(&self);

    fn initialize_task(&self, task: &mut Task) {
        let new_stack_pointer = self.initialize_stack(
            task.stack_pointer(),
            task.entry_point(),
            task.entry_param(),
            0,
        );

        task.set_stack_pointer(new_stack_pointer);
        task.set_ready();
    }
}
