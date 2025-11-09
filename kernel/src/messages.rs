use crate::messages::KeyboardEvent::Touch;
use alloc::fmt::Debug;

#[derive(Debug)]
pub enum HardwareInterrupt {
    Keyboard { scancode: u8 },
}

pub enum KeyboardEvent {
    Touch { key: u8 },
}

struct HardwareInterruptMapper {}

impl HardwareInterruptMapper {
    fn from_keyboard(hardware_interrupt: HardwareInterrupt) -> KeyboardEvent {
        match hardware_interrupt {
            HardwareInterrupt::Keyboard { scancode } => Touch { key: scancode },
        }
    }
}
