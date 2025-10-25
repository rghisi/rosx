use alloc::fmt::Debug;

#[derive(Debug)]
pub enum HardwareInterrupt {
    Keyboard { scancode: u8},
}
