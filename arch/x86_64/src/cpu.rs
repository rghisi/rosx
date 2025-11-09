use crate::interrupts::SYSTEM_TIME_MS;
use core::arch::asm;
use core::sync::atomic::Ordering::Relaxed;
use kernel::cpu::Cpu;
use system::message::Message;

pub struct X86_64 {}

impl X86_64 {
    pub const fn new() -> Self {
        X86_64 {}
    }
}

impl Cpu for X86_64 {
    fn setup(&self) {
        crate::interrupts::init();
        crate::interrupts::enable_timer();
    }

    fn enable_interrupts(&self) {
        crate::interrupts::enable_interrupts();
    }

    fn disable_interrupts(&self) {
        // crate::interrupts::disable_interrupts();
    }

    fn initialize_stack(
        &self,
        stack_pointer: usize,
        entry_point: usize,
        param1: usize,
        param2: usize,
    ) -> usize {
        unsafe {
            let mut sp = stack_pointer as *mut usize;
            sp = sp.sub(1);
            *sp = entry_point;
            sp = sp.sub(1);
            *sp = 0x15; //r15
            sp = sp.sub(1);
            *sp = 0x14; //r14
            sp = sp.sub(1);
            *sp = 0x13; //r13
            sp = sp.sub(1);
            *sp = 0x12; //r12
            sp = sp.sub(1);
            *sp = 0x11; //r11
            sp = sp.sub(1);
            *sp = 0x10; //r10
            sp = sp.sub(1);
            *sp = 0x09; //r09
            sp = sp.sub(1);
            *sp = 0x08; //r08
            sp = sp.sub(1);
            *sp = param1; //rdi
            sp = sp.sub(1);
            *sp = param2; //rsi
            sp = sp.sub(1);
            *sp = 0x0b; //rbp
            sp = sp.sub(1);
            *sp = 0x0c; //rdx
            sp = sp.sub(1);
            *sp = 0x0d; //rcx
            sp = sp.sub(1);
            *sp = 0x0e; //rbx
            sp = sp.sub(1);
            *sp = 0x0f; //rax

            sp as usize
        }
    }

    #[inline(always)]
    fn swap_context(&self, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize) {
        unsafe {
            swap_context(stack_pointer_to_store, stack_pointer_to_load);
        }
    }

    fn syscall(&self, message: &Message) -> usize {
        let result: usize;
        let message_ptr: usize = message as *const Message as usize;
        let param_b: usize = 0xFACADA;

        unsafe {
            asm!(
                "int 0x80",
                out("rax") result,
                in("rdi") message_ptr,
                in("rsi") param_b,
            );
        }

        result
    }

    #[inline(always)]
    fn get_system_time(&self) -> u64 {
        SYSTEM_TIME_MS.load(Relaxed)
    }
}

core::arch::global_asm!(include_str!("context_switching.S"));
core::arch::global_asm!(include_str!("syscall.S"));

unsafe extern "C" {
    pub fn swap_context(stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize);
    pub fn syscall_handler_entry(param_a: usize, param_b: usize) -> usize;
}

#[unsafe(no_mangle)]
unsafe extern "C" fn syscall_handler(param_a: usize, param_b: usize) -> usize {
    let message = &*(param_a as *const Message);
    kernel::syscall::handle_syscall(message)
}
