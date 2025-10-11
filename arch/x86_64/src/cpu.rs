use kernel::cpu::{Cpu};
use kernel::kprintln;

pub struct X86_64 {}

impl Cpu for X86_64 {
    fn setup(&self) {
        crate::interrupts::init();
        crate::interrupts::enable_timer();
    }

    fn enable_interrupts(&self) {
        crate::interrupts::enable_interrupts();
    }

    fn disable_interrupts(&self) {
    }

    fn setup_sys_ticks(&self) {
    }

    fn initialize_task(&self, stack_pointer: usize, entry_point: usize, entry_param: usize) -> usize {
        unsafe {
            initialize_task_for_swap(stack_pointer, entry_point, entry_param)
        }
    }

    fn swap_context(&self, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize) {
        unsafe {
            swap_context(stack_pointer_to_store, stack_pointer_to_load);
        }
    }

    fn trigger_yield(&self) {
        kprintln!("[TEST] About to trigger yield interrupt (INT 0x30)...");
        unsafe {
            core::arch::asm!("int 0x30");
        }
        kprintln!("[TEST] Returned from yield interrupt");
    }
}

// #[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("process_initialization.S"));
core::arch::global_asm!(include_str!("context_switching.S"));

unsafe extern "C" {
    /// Calls the assembly function defined in process_initialization.S
    /// Returns the initial RSP value.
    pub fn initialize_process_stack(stack_top: usize, entry_point: usize, rflags: usize) -> usize;

    /// Calls the assembly function defined in process_initialization.S
    /// Initializes a task's stack for use with swap_context.
    /// Returns the initial RSP value.
    pub fn initialize_task_for_swap(stack_top: usize, entry_point: usize, entry_param: usize) -> usize;

    /// Calls the assembly function defined in context_switching.S
    /// Saves the current context to the location pointed by stack_pointer_to_store,
    /// then loads and restores the context from stack_pointer_to_load.
    pub fn swap_context(stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize);
}
