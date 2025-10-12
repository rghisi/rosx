use core::arch::naked_asm;
use kernel::cpu::{Cpu};
use kernel::kprintln;

pub struct X86_64 {}

impl Cpu for X86_64 {
    fn setup(&self) {
        crate::interrupts::init();
        // crate::interrupts::enable_timer();
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
            let mut sp = stack_pointer as *mut usize;
            sp = sp.sub(1);
            *sp = entry_point;
            sp = sp.sub(1);
            *sp = 0x15;             //r15
            sp = sp.sub(1);
            *sp = 0x14;             //r14
            sp = sp.sub(1);
            *sp = 0x13;             //r13
            sp = sp.sub(1);
            *sp = 0x12;             //r12
            sp = sp.sub(1);
            *sp = 0x11;             //r11
            sp = sp.sub(1);
            *sp = 0x10;             //r10
            sp = sp.sub(1);
            *sp = 0x09;             //r09
            sp = sp.sub(1);
            *sp = 0x08;             //r08
            sp = sp.sub(1);
            *sp = entry_param;      //rdi
            sp = sp.sub(1);
            *sp = 0xa;      //rsi
            sp = sp.sub(1);
            *sp = 0x0b;             //rbp
            sp = sp.sub(1);
            *sp = 0x0c;      //rdx
            sp = sp.sub(1);
            *sp = 0x0d;             //rcx
            sp = sp.sub(1);
            *sp = 0x0e;             //rbx
            sp = sp.sub(1);
            *sp = 0x0f;             //rax

            sp as usize
        }
    }

    fn swap_context(&self, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize) {
        unsafe {
            swap_context(stack_pointer_to_store, stack_pointer_to_load);
        }
    }

    fn trigger_yield(&self) {
        unsafe {
            core::arch::asm!("int 0x30");
        }
    }
}

// #[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("process_initialization.S"));
core::arch::global_asm!(include_str!("context_switching.S"));

unsafe extern "C" {
    /// Calls the assembly function defined in process_initialization.S
    /// Initializes a task's stack for interrupt-driven context switching.
    /// Creates a fake interrupt frame (15 GPRs + RIP + CS + RFLAGS).
    /// Returns the initial RSP value pointing to the base of the interrupt frame.
    pub fn initialize_task_stack(stack_top: usize, entry_point: usize, entry_param: usize) -> usize;

    pub fn swap_context(stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize);
}
