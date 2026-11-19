use input_protocol::wire::{
    is_arrow_sentinel, KEY_ARROW_DOWN, KEY_ARROW_LEFT, KEY_ARROW_RIGHT, KEY_ARROW_UP,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Insert(char),
    Backspace,
    Enter,
    Tab,
    Delete,
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    Save,
    Quit,
}

pub fn command_for(c: char) -> Option<Command> {
    match c {
        '\n' => Some(Command::Enter),
        '\x08' => Some(Command::Backspace),
        '\t' => Some(Command::Tab),
        '\x01' => Some(Command::LineStart),
        '\x02' => Some(Command::Left),
        '\x04' => Some(Command::Delete),
        '\x05' => Some(Command::LineEnd),
        '\x06' => Some(Command::Right),
        '\x0e' => Some(Command::Down),
        '\x10' => Some(Command::Up),
        '\x11' => Some(Command::Quit),
        '\x13' => Some(Command::Save),
        '\x7f' => Some(Command::Delete),
        KEY_ARROW_LEFT => Some(Command::Left),
        KEY_ARROW_RIGHT => Some(Command::Right),
        KEY_ARROW_UP => Some(Command::Up),
        KEY_ARROW_DOWN => Some(Command::Down),
        c if c < '\u{20}' || is_arrow_sentinel(c) => None,
        c => Some(Command::Insert(c)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_protocol::wire::KEY_ESCAPE;

    #[test]
    fn editing_keys_map_to_commands() {
        assert_eq!(command_for('a'), Some(Command::Insert('a')));
        assert_eq!(command_for('~'), Some(Command::Insert('~')));
        assert_eq!(command_for('\n'), Some(Command::Enter));
        assert_eq!(command_for('\x08'), Some(Command::Backspace));
        assert_eq!(command_for('\t'), Some(Command::Tab));
    }

    #[test]
    fn control_codes_map_to_the_pico_keymap() {
        assert_eq!(command_for('\x01'), Some(Command::LineStart));
        assert_eq!(command_for('\x02'), Some(Command::Left));
        assert_eq!(command_for('\x04'), Some(Command::Delete));
        assert_eq!(command_for('\x05'), Some(Command::LineEnd));
        assert_eq!(command_for('\x06'), Some(Command::Right));
        assert_eq!(command_for('\x0e'), Some(Command::Down));
        assert_eq!(command_for('\x10'), Some(Command::Up));
        assert_eq!(command_for('\x11'), Some(Command::Quit));
        assert_eq!(command_for('\x13'), Some(Command::Save));
    }

    #[test]
    fn arrows_map_to_movement_commands() {
        assert_eq!(command_for(KEY_ARROW_UP), Some(Command::Up));
        assert_eq!(command_for(KEY_ARROW_DOWN), Some(Command::Down));
        assert_eq!(command_for(KEY_ARROW_LEFT), Some(Command::Left));
        assert_eq!(command_for(KEY_ARROW_RIGHT), Some(Command::Right));
    }

    #[test]
    fn unmapped_control_keys_and_escape_produce_nothing() {
        assert_eq!(command_for(KEY_ESCAPE), None);
        assert_eq!(command_for('\x03'), None);
        assert_eq!(command_for('\x07'), None);
        assert_eq!(command_for('\x19'), None);
    }
}
