#![cfg_attr(not(test), no_std)]

pub mod crc32;
pub mod serial;

mod pio;

pub const COM1_PORT: u16 = 0x3F8;

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
