use alloc::boxed::Box;
use kernel::cpu::{Cpu};
use kernel::function_task::FunctionTask;
use kernel::task::Task;

pub struct X86_64 {}

impl Cpu for X86_64 {
    fn setup(&self) {
    }

    fn enable_interrupts(&self) {
    }

    fn disable_interrupts(&self) {
    }

    fn setup_sys_ticks(&self) {
    }

    fn initialize_task(&self, mut task: Box<Task>) -> Box<Task> {
        unsafe {
            let new_rsp = initialize_process_stack(task.stack_pointer(), task.entry_point());
            task.set_stack_pointer(new_rsp);
            task
        }
    }

    fn swap_context(&self, stack_pointer_to_store: *mut u8, stack_pointer_to_load: *mut u8) {
    }
}

// #[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("process_initialization.S"));

unsafe extern "C" {
    /// Calls the assembly function defined in context.S
    /// Returns the initial RSP value.
    pub fn initialize_process_stack(stack_top: usize, entry_point: usize) -> usize;
}
