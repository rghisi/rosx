use crate::interrupts::SYSTEM_TIME_MS;
use core::arch::asm;
use core::sync::atomic::Ordering::Relaxed;
use kernel::cpu::Cpu;
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask, Star};
use x86_64::structures::gdt::SegmentSelector;
use x86_64::VirtAddr;

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
        crate::interrupts::enable_keyboard();

        unsafe {
            // Enable System Call Extension (SCE) in EFER.
            // Disable No-Execute Enable (NXE): bootloader 0.11 sets NXE and maps the
            // physical memory region (kernel heap) as non-executable. ELF images are
            // loaded into heap memory, so clearing NXE makes all pages executable.
            Efer::update(|flags| {
                flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
                flags.remove(EferFlags::NO_EXECUTE_ENABLE);
            });

            // Set the entry point for syscalls
            LStar::write(VirtAddr::new(syscall_handler_entry as *const () as u64));

            // Configure segments in STAR
            // Bootloader v0.9 GDT:
            // 0: Null
            // 1: Kernel Code (0x08)
            // 2: Kernel Data (0x10)
            // 3: User Data (0x18)
            // 4: User Code (0x20)
            Star::write(
                SegmentSelector::new(4, x86_64::PrivilegeLevel::Ring3), // User Code
                SegmentSelector::new(3, x86_64::PrivilegeLevel::Ring3), // User Data
                SegmentSelector::new(1, x86_64::PrivilegeLevel::Ring0), // Kernel Code
                SegmentSelector::new(2, x86_64::PrivilegeLevel::Ring0), // Kernel Data
            )
            .unwrap();

            // Mask interrupts on syscall entry
            SFMask::write(x86_64::registers::rflags::RFlags::INTERRUPT_FLAG);
        }
    }

    fn enable_interrupts(&self) {
        x86_64::instructions::interrupts::enable();
    }

    fn disable_interrupts(&self) {
        x86_64::instructions::interrupts::disable();
    }

    fn are_interrupts_enabled(&self) -> bool {
        x86_64::registers::rflags::read().contains(x86_64::registers::rflags::RFlags::INTERRUPT_FLAG)
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
            
            // Align stack pointer to 16 bytes for ABI compliance
            let sp_val = sp as usize;
            sp = (sp_val & !0xF) as *mut usize;

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

    #[inline(always)]
    fn get_system_time(&self) -> u64 {
        SYSTEM_TIME_MS.load(Relaxed)
    }

    fn halt(&self) {
        unsafe {
            asm!("hlt");
        }
    }
}

pub unsafe fn clear_nx_bits(phys_offset: u64) {
    use x86_64::registers::control::Cr3;
    let (frame, flags) = Cr3::read();
    walk_page_table(frame.start_address().as_u64(), phys_offset, 4);
    Cr3::write(frame, flags);
}

unsafe fn walk_page_table(table_phys: u64, phys_offset: u64, level: u8) {
    let table = (table_phys + phys_offset) as *mut u64;
    for i in 0usize..512 {
        let entry = table.add(i).read();
        if entry & 1 == 0 {
            continue;
        }
        table.add(i).write(entry & !(1u64 << 63));
        if level > 1 && entry & (1 << 7) == 0 {
            walk_page_table(entry & 0x000f_ffff_ffff_f000, phys_offset, level - 1);
        }
    }
}

core::arch::global_asm!(include_str!("context_switching.S"));
core::arch::global_asm!(include_str!("syscall.S"));

unsafe extern "C" {
    pub fn swap_context(stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize);
    pub fn syscall_handler_entry();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn syscall_handler(num: u64, arg1: u64, arg2: u64, arg3: u64) -> usize {
    kernel::syscall::handle_syscall(num, arg1, arg2, arg3)
}
