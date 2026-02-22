use crate::future::Future;
use crate::kernel_cell::KernelCell;
use alloc::collections::VecDeque;
use alloc::fmt::{Display, Formatter};
use lazy_static::lazy_static;

lazy_static! {
    static ref KEYBOARD_BUFFER: KernelCell<VecDeque<char>> = KernelCell::new(VecDeque::new());
}

pub fn push_key(c: char) {
    KEYBOARD_BUFFER.borrow_mut().push_back(c);
}

pub fn pop_key() -> Option<char> {
    KEYBOARD_BUFFER.borrow_mut().pop_front()
}

pub struct KeyboardFuture {}

impl KeyboardFuture {
    pub fn new() -> Self {
        Self {}
    }
}

impl Future for KeyboardFuture {
    fn is_completed(&self) -> bool {
        !KEYBOARD_BUFFER.borrow_mut().is_empty()
    }
}

#[derive(Debug)]
pub enum Key {
    Escape,
    Key1,
    Key2,
    Key3,
    Key4,
    Key5,
    Key6,
    Key7,
    Key8,
    Key9,
    Key0,
    Minus,
    Equals,
    Backspace,
    Tab,
    Q,
    W,
    E,
    R,
    T,
    Y,
    U,
    I,
    O,
    P,
    LeftBracket,
    RightBracket,
    Enter, // Main Enter
    LeftControl,
    A,
    S,
    D,
    F,
    G,
    H,
    J,
    K,
    L,
    Semicolon,
    Apostrophe,
    Grave, // `~
    LeftShift,
    Backslash, // | \
    Z,
    X,
    C,
    V,
    B,
    N,
    M,
    Comma,
    Period,
    Slash, // / ?
    RightShift,
    KeypadAsterisk,
    LeftAlt,
    Spacebar,
    CapsLock,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    NumLock,
    ScrollLock,
    Keypad7, // Home
    Keypad8, // Up
    Keypad9, // Page Up
    KeypadMinus,
    Keypad4, // Left
    Keypad5, // Center
    Keypad6, // Right
    KeypadPlus,
    Keypad1,      // End
    Keypad2,      // Down
    Keypad3,      // Page Down
    Keypad0,      // Insert
    KeypadPeriod, // Delete
    // 0x54 - SysReq (Alt+PrtSc) - often special handling or a multi-byte sequence
    F11,
    F12,
}
impl Key {
    pub fn from_scancode_set1(value: u8) -> Result<Self, u8> {
        let value = value & 0x7F;
        match value {
            0x01 => Ok(Key::Escape),
            0x02 => Ok(Key::Key1),
            0x03 => Ok(Key::Key2),
            0x04 => Ok(Key::Key3),
            0x05 => Ok(Key::Key4),
            0x06 => Ok(Key::Key5),
            0x07 => Ok(Key::Key6),
            0x08 => Ok(Key::Key7),
            0x09 => Ok(Key::Key8),
            0x0A => Ok(Key::Key9),
            0x0B => Ok(Key::Key0),
            0x0C => Ok(Key::Minus),
            0x0D => Ok(Key::Equals),
            0x0E => Ok(Key::Backspace),
            0x0F => Ok(Key::Tab),
            0x10 => Ok(Key::Q),
            0x11 => Ok(Key::W),
            0x12 => Ok(Key::E),
            0x13 => Ok(Key::R),
            0x14 => Ok(Key::T),
            0x15 => Ok(Key::Y),
            0x16 => Ok(Key::U),
            0x17 => Ok(Key::I),
            0x18 => Ok(Key::O),
            0x19 => Ok(Key::P),
            0x1A => Ok(Key::LeftBracket),
            0x1B => Ok(Key::RightBracket),
            0x1C => Ok(Key::Enter),
            0x1D => Ok(Key::LeftControl),
            0x1E => Ok(Key::A),
            0x1F => Ok(Key::S),
            0x20 => Ok(Key::D),
            0x21 => Ok(Key::F),
            0x22 => Ok(Key::G),
            0x23 => Ok(Key::H),
            0x24 => Ok(Key::J),
            0x25 => Ok(Key::K),
            0x26 => Ok(Key::L),
            0x27 => Ok(Key::Semicolon),
            0x28 => Ok(Key::Apostrophe),
            0x29 => Ok(Key::Grave),
            0x2A => Ok(Key::LeftShift),
            0x2B => Ok(Key::Backslash),
            0x2C => Ok(Key::Z),
            0x2D => Ok(Key::X),
            0x2E => Ok(Key::C),
            0x2F => Ok(Key::V),
            0x30 => Ok(Key::B),
            0x31 => Ok(Key::N),
            0x32 => Ok(Key::M),
            0x33 => Ok(Key::Comma),
            0x34 => Ok(Key::Period),
            0x35 => Ok(Key::Slash),
            0x36 => Ok(Key::RightShift),
            0x37 => Ok(Key::KeypadAsterisk),
            0x38 => Ok(Key::LeftAlt),
            0x39 => Ok(Key::Spacebar),
            0x3A => Ok(Key::CapsLock),
            0x3B => Ok(Key::F1),
            0x3C => Ok(Key::F2),
            0x3D => Ok(Key::F3),
            0x3E => Ok(Key::F4),
            0x3F => Ok(Key::F5),
            0x40 => Ok(Key::F6),
            0x41 => Ok(Key::F7),
            0x42 => Ok(Key::F8),
            0x43 => Ok(Key::F9),
            0x44 => Ok(Key::F10),
            0x45 => Ok(Key::NumLock),
            0x46 => Ok(Key::ScrollLock),
            0x47 => Ok(Key::Keypad7),
            0x48 => Ok(Key::Keypad8),
            0x49 => Ok(Key::Keypad9),
            0x4A => Ok(Key::KeypadMinus),
            0x4B => Ok(Key::Keypad4),
            0x4C => Ok(Key::Keypad5),
            0x4D => Ok(Key::Keypad6),
            0x4E => Ok(Key::KeypadPlus),
            0x4F => Ok(Key::Keypad1),
            0x50 => Ok(Key::Keypad2),
            0x51 => Ok(Key::Keypad3),
            0x52 => Ok(Key::Keypad0),
            0x53 => Ok(Key::KeypadPeriod),
            0x57 => Ok(Key::F11),
            0x58 => Ok(Key::F12),
            _ => Err(value),
        }
    }
}
#[derive(Debug)]
pub struct KeyboardEvent {
    pub char: Option<char>,
}

impl KeyboardEvent {
    pub fn from_key(key: Key) -> KeyboardEvent {
        match key {
            Key::Escape => KeyboardEvent { char: None },
            Key::Key1 => KeyboardEvent { char: Some('1') },
            Key::Key2 => KeyboardEvent { char: Some('2') },
            Key::Key3 => KeyboardEvent { char: Some('3') },
            Key::Key4 => KeyboardEvent { char: Some('4') },
            Key::Key5 => KeyboardEvent { char: Some('5') },
            Key::Key6 => KeyboardEvent { char: Some('6') },
            Key::Key7 => KeyboardEvent { char: Some('7') },
            Key::Key8 => KeyboardEvent { char: Some('8') },
            Key::Key9 => KeyboardEvent { char: Some('9') },
            Key::Key0 => KeyboardEvent { char: Some('0') },
            Key::Minus => KeyboardEvent { char: Some('-') },
            Key::Equals => KeyboardEvent { char: Some('=') },
            Key::Backspace => KeyboardEvent { char: Some('\x08') },
            Key::Tab => KeyboardEvent { char: Some('\t') },
            Key::Q => KeyboardEvent { char: Some('q') },
            Key::W => KeyboardEvent { char: Some('w') },
            Key::E => KeyboardEvent { char: Some('e') },
            Key::R => KeyboardEvent { char: Some('r') },
            Key::T => KeyboardEvent { char: Some('t') },
            Key::Y => KeyboardEvent { char: Some('y') },
            Key::U => KeyboardEvent { char: Some('u') },
            Key::I => KeyboardEvent { char: Some('i') },
            Key::O => KeyboardEvent { char: Some('o') },
            Key::P => KeyboardEvent { char: Some('p') },
            Key::LeftBracket => KeyboardEvent { char: Some('[') },
            Key::RightBracket => KeyboardEvent { char: Some(']') },
            Key::Enter => KeyboardEvent { char: Some('\n') },
            Key::LeftControl => KeyboardEvent { char: None },
            Key::A => KeyboardEvent { char: Some('a') },
            Key::S => KeyboardEvent { char: Some('s') },
            Key::D => KeyboardEvent { char: Some('d') },
            Key::F => KeyboardEvent { char: Some('f') },
            Key::G => KeyboardEvent { char: Some('g') },
            Key::H => KeyboardEvent { char: Some('h') },
            Key::J => KeyboardEvent { char: Some('j') },
            Key::K => KeyboardEvent { char: Some('k') },
            Key::L => KeyboardEvent { char: Some('l') },
            Key::Semicolon => KeyboardEvent { char: Some(';') },
            Key::Apostrophe => KeyboardEvent { char: Some('\'') },
            Key::Grave => KeyboardEvent { char: None },
            Key::LeftShift => KeyboardEvent { char: None },
            Key::Backslash => KeyboardEvent { char: Some('\\') },
            Key::Z => KeyboardEvent { char: Some('z') },
            Key::X => KeyboardEvent { char: Some('x') },
            Key::C => KeyboardEvent { char: Some('c') },
            Key::V => KeyboardEvent { char: Some('v') },
            Key::B => KeyboardEvent { char: Some('b') },
            Key::N => KeyboardEvent { char: Some('n') },
            Key::M => KeyboardEvent { char: Some('m') },
            Key::Comma => KeyboardEvent { char: Some(',') },
            Key::Period => KeyboardEvent { char: Some('.') },
            Key::Slash => KeyboardEvent { char: Some('/') },
            Key::RightShift => KeyboardEvent { char: None },
            Key::KeypadAsterisk => KeyboardEvent { char: None },
            Key::LeftAlt => KeyboardEvent { char: None },
            Key::Spacebar => KeyboardEvent { char: Some(' ') },
            Key::CapsLock => KeyboardEvent { char: None },
            Key::F1 => KeyboardEvent { char: None },
            Key::F2 => KeyboardEvent { char: None },
            Key::F3 => KeyboardEvent { char: None },
            Key::F4 => KeyboardEvent { char: None },
            Key::F5 => KeyboardEvent { char: None },
            Key::F6 => KeyboardEvent { char: None },
            Key::F7 => KeyboardEvent { char: None },
            Key::F8 => KeyboardEvent { char: None },
            Key::F9 => KeyboardEvent { char: None },
            Key::F10 => KeyboardEvent { char: None },
            Key::NumLock => KeyboardEvent { char: None },
            Key::ScrollLock => KeyboardEvent { char: None },
            Key::Keypad7 => KeyboardEvent { char: None },
            Key::Keypad8 => KeyboardEvent { char: None },
            Key::Keypad9 => KeyboardEvent { char: None },
            Key::KeypadMinus => KeyboardEvent { char: None },
            Key::Keypad4 => KeyboardEvent { char: None },
            Key::Keypad5 => KeyboardEvent { char: None },
            Key::Keypad6 => KeyboardEvent { char: None },
            Key::KeypadPlus => KeyboardEvent { char: None },
            Key::Keypad1 => KeyboardEvent { char: None },
            Key::Keypad2 => KeyboardEvent { char: None },
            Key::Keypad3 => KeyboardEvent { char: None },
            Key::Keypad0 => KeyboardEvent { char: None },
            Key::KeypadPeriod => KeyboardEvent { char: None },
            Key::F11 => KeyboardEvent { char: None },
            Key::F12 => KeyboardEvent { char: None },
        }
    }
}

impl Display for KeyboardEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> alloc::fmt::Result {
        let char = if self.char.is_some() {
            self.char.unwrap()
        } else {
            ' '
        };
        write!(f, "KeyboardEvent: {}", char)
    }
}
