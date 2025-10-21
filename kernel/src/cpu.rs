use alloc::boxed::Box;
use task::Task;

pub trait Cpu {
    fn setup(&self);
    fn enable_interrupts(&self);
    fn disable_interrupts(&self);
    fn setup_sys_ticks(&self);
    fn initialize_stack(&self, stack_pointer: usize, entry_point: usize, entry_param: usize) -> usize;
    fn swap_context(&self, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize);
    fn trigger_yield(&self);

    fn initialize_task(&self, task: &mut Task) {
        let original_sp = task.stack_pointer();

        let new_stack_pointer = self.initialize_stack(
            original_sp,
            task.entry_point(),
            task.actual_entry_point()
        );

        task.set_stack_pointer(new_stack_pointer);
        task.set_ready();
    }
}
