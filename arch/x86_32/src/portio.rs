use core::arch::asm;
use kernel::platform::portio::{PortIo, PortWidth};

pub struct X86_32PortIo;

impl PortIo for X86_32PortIo {
    fn read(&self, port: u16, width: PortWidth) -> u32 {
        match width {
            PortWidth::Byte => unsafe { read_byte(port) as u32 },
            PortWidth::Word => unsafe { read_word(port) as u32 },
            PortWidth::DWord => unsafe { read_dword(port) },
        }
    }

    fn write(&self, port: u16, value: u32, width: PortWidth) {
        match width {
            PortWidth::Byte => unsafe { write_byte(port, value as u8) },
            PortWidth::Word => unsafe { write_word(port, value as u16) },
            PortWidth::DWord => unsafe { write_dword(port, value) },
        }
    }

    fn read_words(&self, port: u16, destination: &mut [u16]) {
        for word in destination.iter_mut() {
            *word = unsafe { read_word(port) };
        }
    }

    fn write_words(&self, port: u16, source: &[u16]) {
        for word in source {
            unsafe { write_word(port, *word) };
        }
    }
}

unsafe fn read_byte(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", out("al") value, in("dx") port,
             options(nomem, nostack, preserves_flags));
    }
    value
}

unsafe fn read_word(port: u16) -> u16 {
    let value: u16;
    unsafe {
        asm!("in ax, dx", out("ax") value, in("dx") port,
             options(nomem, nostack, preserves_flags));
    }
    value
}

unsafe fn read_dword(port: u16) -> u32 {
    let value: u32;
    unsafe {
        asm!("in eax, dx", out("eax") value, in("dx") port,
             options(nomem, nostack, preserves_flags));
    }
    value
}

unsafe fn write_byte(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value,
             options(nomem, nostack, preserves_flags));
    }
}

unsafe fn write_word(port: u16, value: u16) {
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") value,
             options(nomem, nostack, preserves_flags));
    }
}

unsafe fn write_dword(port: u16, value: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value,
             options(nomem, nostack, preserves_flags));
    }
}
