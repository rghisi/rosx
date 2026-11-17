#![cfg_attr(not(test), no_std)]

pub mod bios;
pub mod biosdisk;
pub mod bootinfo;
pub mod crc32;
pub mod debugport;
pub mod elf32;
pub mod elf64;
pub mod gdt;
pub mod memnorm;
pub mod pagetable;
pub mod serial;

mod pio;

pub const COM1_PORT: u16 = 0x3F8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ElfError {
    Class,
    Machine,
    Format,
    Layout,
    Entry,
}

pub const DEBUG_EXIT_PORT: u16 = 0xF4;
pub const DEBUG_EXIT_SUCCESS: u8 = 0x10;
pub const DEBUG_EXIT_FAILURE: u8 = 0x01;

pub fn debug_exit(status: u8) -> ! {
    pio::outb(DEBUG_EXIT_PORT, status);
    halt_forever()
}

pub fn halt_forever() -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(preserves_flags));
        }
    }
}
