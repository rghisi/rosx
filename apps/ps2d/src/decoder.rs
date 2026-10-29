pub struct Ps2Decoder {
    shifted: bool,
}

impl Ps2Decoder {
    pub fn new() -> Ps2Decoder {
        Ps2Decoder { shifted: false }
    }

    pub fn feed(&mut self, scancode: u8) -> Option<char> {
        match scancode {
            0x2A | 0x36 => {
                self.shifted = true;
                None
            }
            0xAA | 0xB6 => {
                self.shifted = false;
                None
            }
            raw if raw & 0x80 != 0 => None,
            raw => Key::from_scancode_set1(raw)
                .ok()
                .and_then(|key| char_for(key, self.shifted)),
        }
    }
}

impl Default for Ps2Decoder {
    fn default() -> Ps2Decoder {
        Ps2Decoder::new()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
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
    Enter,
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
    Grave,
    LeftShift,
    Backslash,
    Z,
    X,
    C,
    V,
    B,
    N,
    M,
    Comma,
    Period,
    Slash,
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
    Keypad7,
    Keypad8,
    Keypad9,
    KeypadMinus,
    Keypad4,
    Keypad5,
    Keypad6,
    KeypadPlus,
    Keypad1,
    Keypad2,
    Keypad3,
    Keypad0,
    KeypadPeriod,
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

fn char_for(key: Key, shifted: bool) -> Option<char> {
    match key {
        Key::Escape => None,
        Key::Key1 => Some(if shifted { '!' } else { '1' }),
        Key::Key2 => Some(if shifted { '@' } else { '2' }),
        Key::Key3 => Some(if shifted { '#' } else { '3' }),
        Key::Key4 => Some(if shifted { '$' } else { '4' }),
        Key::Key5 => Some(if shifted { '%' } else { '5' }),
        Key::Key6 => Some(if shifted { '^' } else { '6' }),
        Key::Key7 => Some(if shifted { '&' } else { '7' }),
        Key::Key8 => Some(if shifted { '*' } else { '8' }),
        Key::Key9 => Some(if shifted { '(' } else { '9' }),
        Key::Key0 => Some(if shifted { ')' } else { '0' }),
        Key::Minus => Some(if shifted { '_' } else { '-' }),
        Key::Equals => Some(if shifted { '+' } else { '=' }),
        Key::Backspace => Some('\x08'),
        Key::Tab => Some('\t'),
        Key::Q => Some(if shifted { 'Q' } else { 'q' }),
        Key::W => Some(if shifted { 'W' } else { 'w' }),
        Key::E => Some(if shifted { 'E' } else { 'e' }),
        Key::R => Some(if shifted { 'R' } else { 'r' }),
        Key::T => Some(if shifted { 'T' } else { 't' }),
        Key::Y => Some(if shifted { 'Y' } else { 'y' }),
        Key::U => Some(if shifted { 'U' } else { 'u' }),
        Key::I => Some(if shifted { 'I' } else { 'i' }),
        Key::O => Some(if shifted { 'O' } else { 'o' }),
        Key::P => Some(if shifted { 'P' } else { 'p' }),
        Key::LeftBracket => Some(if shifted { '{' } else { '[' }),
        Key::RightBracket => Some(if shifted { '}' } else { ']' }),
        Key::Enter => Some('\n'),
        Key::LeftControl => None,
        Key::A => Some(if shifted { 'A' } else { 'a' }),
        Key::S => Some(if shifted { 'S' } else { 's' }),
        Key::D => Some(if shifted { 'D' } else { 'd' }),
        Key::F => Some(if shifted { 'F' } else { 'f' }),
        Key::G => Some(if shifted { 'G' } else { 'g' }),
        Key::H => Some(if shifted { 'H' } else { 'h' }),
        Key::J => Some(if shifted { 'J' } else { 'j' }),
        Key::K => Some(if shifted { 'K' } else { 'k' }),
        Key::L => Some(if shifted { 'L' } else { 'l' }),
        Key::Semicolon => Some(if shifted { ':' } else { ';' }),
        Key::Apostrophe => Some(if shifted { '"' } else { '\'' }),
        Key::Grave => shifted.then_some('~'),
        Key::LeftShift => None,
        Key::Backslash => Some(if shifted { '|' } else { '\\' }),
        Key::Z => Some(if shifted { 'Z' } else { 'z' }),
        Key::X => Some(if shifted { 'X' } else { 'x' }),
        Key::C => Some(if shifted { 'C' } else { 'c' }),
        Key::V => Some(if shifted { 'V' } else { 'v' }),
        Key::B => Some(if shifted { 'B' } else { 'b' }),
        Key::N => Some(if shifted { 'N' } else { 'n' }),
        Key::M => Some(if shifted { 'M' } else { 'm' }),
        Key::Comma => Some(if shifted { '<' } else { ',' }),
        Key::Period => Some(if shifted { '>' } else { '.' }),
        Key::Slash => Some(if shifted { '?' } else { '/' }),
        Key::RightShift => None,
        Key::KeypadAsterisk => None,
        Key::LeftAlt => None,
        Key::Spacebar => Some(' '),
        Key::CapsLock => None,
        Key::F1 => None,
        Key::F2 => None,
        Key::F3 => None,
        Key::F4 => None,
        Key::F5 => None,
        Key::F6 => None,
        Key::F7 => None,
        Key::F8 => None,
        Key::F9 => None,
        Key::F10 => None,
        Key::NumLock => None,
        Key::ScrollLock => None,
        Key::Keypad7 => None,
        Key::Keypad8 => None,
        Key::Keypad9 => None,
        Key::KeypadMinus => None,
        Key::Keypad4 => None,
        Key::Keypad5 => None,
        Key::Keypad6 => None,
        Key::KeypadPlus => None,
        Key::Keypad1 => None,
        Key::Keypad2 => None,
        Key::Keypad3 => None,
        Key::Keypad0 => None,
        Key::KeypadPeriod => None,
        Key::F11 => None,
        Key::F12 => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(scancodes: &[u8]) -> Vec<char> {
        let mut decoder = Ps2Decoder::new();
        scancodes.iter().filter_map(|s| decoder.feed(*s)).collect()
    }

    #[test]
    fn unshifted_letters_and_digits_match_the_legacy_table() {
        assert_eq!(typed(&[0x1E]), vec!['a']);
        assert_eq!(typed(&[0x10]), vec!['q']);
        assert_eq!(typed(&[0x20]), vec!['d']);
        assert_eq!(typed(&[0x02, 0x03, 0x0B]), vec!['1', '2', '0']);
        assert_eq!(typed(&[0x39]), vec![' ']);
        assert_eq!(typed(&[0x1C]), vec!['\n']);
        assert_eq!(typed(&[0x0E]), vec!['\x08']);
        assert_eq!(typed(&[0x0F]), vec!['\t']);
        assert_eq!(typed(&[0x2C]), vec!['z']);
        assert_eq!(typed(&[0x32]), vec!['m']);
    }

    #[test]
    fn unshifted_punctuation_matches_the_legacy_table() {
        assert_eq!(typed(&[0x0C]), vec!['-']);
        assert_eq!(typed(&[0x0D]), vec!['=']);
        assert_eq!(typed(&[0x1A]), vec!['[']);
        assert_eq!(typed(&[0x1B]), vec![']']);
        assert_eq!(typed(&[0x2B]), vec!['\\']);
        assert_eq!(typed(&[0x27]), vec![';']);
        assert_eq!(typed(&[0x28]), vec!['\'']);
        assert_eq!(typed(&[0x33]), vec![',']);
        assert_eq!(typed(&[0x34]), vec!['.']);
        assert_eq!(typed(&[0x35]), vec!['/']);
    }

    #[test]
    fn deferred_shift_sequence_produces_underscore_only() {
        assert_eq!(typed(&[0x2A, 0x0C, 0xAA]), vec!['_']);
    }

    #[test]
    fn shift_press_and_release_produce_no_char() {
        assert_eq!(typed(&[0x2A]), Vec::<char>::new());
        assert_eq!(typed(&[0x36]), Vec::<char>::new());
        assert_eq!(typed(&[0xAA]), Vec::<char>::new());
        assert_eq!(typed(&[0xB6]), Vec::<char>::new());
    }

    #[test]
    fn shifted_letters_are_uppercase() {
        assert_eq!(typed(&[0x2A, 0x1E, 0xAA]), vec!['A']);
        assert_eq!(typed(&[0x36, 0x10, 0xB6]), vec!['Q']);
    }

    #[test]
    fn shift_overlay_covers_the_punctuation_row() {
        let overlay = typed(&[
            0x2A, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x1A,
            0x1B, 0x2B, 0x27, 0x28, 0x29, 0x33, 0x34, 0x35,
        ]);
        assert_eq!(overlay, "!@#$%^&*()_+{}|:\"~<>?".chars().collect::<Vec<char>>());
    }

    #[test]
    fn release_frames_are_silent() {
        assert_eq!(typed(&[0x9E]), Vec::<char>::new());
        assert_eq!(typed(&[0x82]), Vec::<char>::new());
        assert_eq!(typed(&[0x1E, 0x9E]), vec!['a']);
    }

    #[test]
    fn release_frames_do_not_clear_shift_state() {
        assert_eq!(typed(&[0x2A, 0x1E, 0x9E, 0x1F, 0xAA, 0x1E]), vec!['A', 'S', 'a']);
    }

    #[test]
    fn unknown_scancodes_are_silent() {
        for scancode in [0x56u8, 0x59, 0x5A, 0x5B, 0x60, 0x72, 0x73, 0x74, 0x7D] {
            assert_eq!(typed(&[scancode]), Vec::<char>::new(), "scancode {scancode:#04x}");
        }
    }

    #[test]
    fn extended_prefix_frames_are_silent() {
        assert_eq!(typed(&[0xE0, 0x70, 0xE0, 0xF0]), Vec::<char>::new());
    }

    #[test]
    fn plain_press_after_shift_release_returns_base_char() {
        assert_eq!(typed(&[0x2A, 0xAA, 0x0C]), vec!['-']);
        assert_eq!(typed(&[0x36, 0xB6, 0x1E]), vec!['a']);
    }

    #[test]
    fn non_char_keys_are_silent_shifted_or_not() {
        for scancode in [0x01u8, 0x1D, 0x38, 0x3A, 0x3B, 0x45, 0x47, 0x52, 0x57, 0x58] {
            assert_eq!(typed(&[scancode]), Vec::<char>::new());
            assert_eq!(typed(&[0x2A, scancode]), Vec::<char>::new());
        }
    }

    #[test]
    fn grave_produces_tilde_only_under_shift() {
        assert_eq!(typed(&[0x29]), Vec::<char>::new());
        assert_eq!(typed(&[0x2A, 0x29]), vec!['~']);
    }
}
