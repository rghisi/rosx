use alloc::boxed::Box;
use task::Task;

pub trait Cpu {
    fn setup(&self);
    fn enable_interrupts(&self);
    fn disable_interrupts(&self);
    fn setup_sys_ticks(&self);
    fn initialize_task(&self, stack_pointer: usize, entry_point: usize, entry_param: usize) -> usize;
    fn swap_context(&self, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize);
    fn trigger_yield(&self);
}
