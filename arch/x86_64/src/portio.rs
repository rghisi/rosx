use kernel::portio::{PortIo, PortWidth};
use x86_64::instructions::port::Port;

pub struct X86_64PortIo;

impl PortIo for X86_64PortIo {
    fn read(&self, port: u16, width: PortWidth) -> u32 {
        unsafe {
            match width {
                PortWidth::Byte => Port::<u8>::new(port).read() as u32,
                PortWidth::Word => Port::<u16>::new(port).read() as u32,
                PortWidth::DWord => Port::<u32>::new(port).read(),
            }
        }
    }

    fn write(&self, port: u16, value: u32, width: PortWidth) {
        unsafe {
            match width {
                PortWidth::Byte => Port::<u8>::new(port).write(value as u8),
                PortWidth::Word => Port::<u16>::new(port).write(value as u16),
                PortWidth::DWord => Port::<u32>::new(port).write(value),
            }
        }
    }

    fn read_words(&self, port: u16, destination: &mut [u16]) {
        let mut data_port = Port::<u16>::new(port);
        for word in destination.iter_mut() {
            *word = unsafe { data_port.read() };
        }
    }

    fn write_words(&self, port: u16, source: &[u16]) {
        let mut data_port = Port::<u16>::new(port);
        for word in source {
            unsafe { data_port.write(*word) };
        }
    }
}
