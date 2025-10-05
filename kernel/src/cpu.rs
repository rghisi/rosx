use alloc::boxed::Box;
use task::Task;

pub trait Cpu {
    fn setup(&self);
    fn enable_interrupts(&self);
    fn disable_interrupts(&self);
    fn setup_sys_ticks(&self);
    fn initialize_task(&self, task: Box<Task>) -> Box<Task>;
    fn swap_context(&self, stack_pointer_to_store: *mut u8, stack_pointer_to_load: *mut u8);
}
