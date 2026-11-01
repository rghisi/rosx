#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiState {
    Normal,
    Escape,
    Csi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiCommand {
    PrintChar(u8),
    SetForeground(AnsiColor),
    SetBackground(AnsiColor),
    ResetAttributes,
    SetCursorPos { row: usize, col: usize },
    ClearScreen,
    ClearLine,
}

pub struct AnsiParser {
    state: AnsiState,
    params: [u16; 8],
    param_idx: usize,
    current_param: u16,
    has_param: bool,
    command_queue: [Option<AnsiCommand>; 16],
    queue_head: usize,
    queue_tail: usize,
}

impl AnsiParser {
    pub const fn new() -> Self {
        Self {
            state: AnsiState::Normal,
            params: [0; 8],
            param_idx: 0,
            current_param: 0,
            has_param: false,
            command_queue: [None; 16],
            queue_head: 0,
            queue_tail: 0,
        }
    }

    fn push_command(&mut self, command: AnsiCommand) {
        let next_tail = (self.queue_tail + 1) % self.command_queue.len();
        if next_tail != self.queue_head {
            self.command_queue[self.queue_tail] = Some(command);
            self.queue_tail = next_tail;
        }
    }

    pub fn handle_byte(&mut self, byte: u8) {
        match self.state {
            AnsiState::Normal => {
                if byte == 0x1B {
                    self.state = AnsiState::Escape;
                } else {
                    self.push_command(AnsiCommand::PrintChar(byte));
                }
            }
            AnsiState::Escape => {
                if byte == b'[' {
                    self.state = AnsiState::Csi;
                    self.params = [0; 8];
                    self.param_idx = 0;
                    self.current_param = 0;
                    self.has_param = false;
                } else {
                    self.state = AnsiState::Normal;
                    self.push_command(AnsiCommand::PrintChar(byte));
                }
            }
            AnsiState::Csi => {
                match byte {
                    b'0'..=b'9' => {
                        self.current_param = self.current_param * 10 + (byte - b'0') as u16;
                        self.has_param = true;
                    }
                    b';' => {
                        if self.param_idx < self.params.len() {
                            self.params[self.param_idx] = self.current_param;
                            self.param_idx += 1;
                        }
                        self.current_param = 0;
                        self.has_param = false;
                    }
                    b'm' => {
                        if self.has_param {
                            self.params[self.param_idx] = self.current_param;
                            self.param_idx += 1;
                        }

                        if self.param_idx == 0 {
                             self.push_command(AnsiCommand::ResetAttributes);
                        } else {
                            for i in 0..self.param_idx {
                                let param = self.params[i];
                                match param {
                                    0 => self.push_command(AnsiCommand::ResetAttributes),
                                    30..=37 => self.push_command(AnsiCommand::SetForeground(self.ansi_color(param - 30))),
                                    40..=47 => self.push_command(AnsiCommand::SetBackground(self.ansi_color(param - 40))),
                                    90..=97 => self.push_command(AnsiCommand::SetForeground(self.ansi_bright_color(param - 90))),
                                    100..=107 => self.push_command(AnsiCommand::SetBackground(self.ansi_bright_color(param - 100))),
                                    _ => {}
                                }
                            }
                        }
                        self.state = AnsiState::Normal;
                    }
                    b'H' | b'f' => {
                        if self.has_param {
                            self.params[self.param_idx] = self.current_param;
                            self.param_idx += 1;
                        }
                        let row = if self.param_idx > 0 { self.params[0].saturating_sub(1) as usize } else { 0 };
                        let col = if self.param_idx > 1 { self.params[1].saturating_sub(1) as usize } else { 0 };
                        self.push_command(AnsiCommand::SetCursorPos { row, col });
                        self.state = AnsiState::Normal;
                    }
                    b'J' => {
                        self.push_command(AnsiCommand::ClearScreen);
                        self.state = AnsiState::Normal;
                    }
                    b'K' => {
                        self.push_command(AnsiCommand::ClearLine);
                        self.state = AnsiState::Normal;
                    }
                    _ => {
                        self.state = AnsiState::Normal;
                    }
                }
            }
        }
    }

    pub fn next_command(&mut self) -> Option<AnsiCommand> {
        if self.queue_head == self.queue_tail {
            None
        } else {
            let command = self.command_queue[self.queue_head].take();
            self.queue_head = (self.queue_head + 1) % self.command_queue.len();
            command
        }
    }

    const fn ansi_color(&self, code: u16) -> AnsiColor {
        match code {
            0 => AnsiColor::Black,
            1 => AnsiColor::Red,
            2 => AnsiColor::Green,
            3 => AnsiColor::Yellow,
            4 => AnsiColor::Blue,
            5 => AnsiColor::Magenta,
            6 => AnsiColor::Cyan,
            7 => AnsiColor::White,
            _ => AnsiColor::White,
        }
    }

    const fn ansi_bright_color(&self, code: u16) -> AnsiColor {
        match code {
            0 => AnsiColor::BrightBlack,
            1 => AnsiColor::BrightRed,
            2 => AnsiColor::BrightGreen,
            3 => AnsiColor::BrightYellow,
            4 => AnsiColor::BrightBlue,
            5 => AnsiColor::BrightMagenta,
            6 => AnsiColor::BrightCyan,
            7 => AnsiColor::BrightWhite,
            _ => AnsiColor::BrightWhite,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn parse(input: &[u8]) -> Vec<AnsiCommand> {
        let mut parser = AnsiParser::new();
        let mut commands = Vec::new();
        for byte in input {
            parser.handle_byte(*byte);
            while let Some(command) = parser.next_command() {
                commands.push(command);
            }
        }
        while let Some(command) = parser.next_command() {
            commands.push(command);
        }
        commands
    }

    #[test]
    fn empty_input_yields_no_commands() {
        assert_eq!(parse(b""), Vec::new());
    }

    #[test]
    fn plain_text_prints_in_order() {
        assert_eq!(
            parse(b"hi"),
            vec![AnsiCommand::PrintChar(b'h'), AnsiCommand::PrintChar(b'i')]
        );
    }

    #[test]
    fn escape_followed_by_regular_byte_prints_that_byte() {
        assert_eq!(parse(b"\x1bA"), vec![AnsiCommand::PrintChar(b'A')]);
    }

    #[test]
    fn sgr_without_params_resets_attributes() {
        assert_eq!(parse(b"\x1b[m"), vec![AnsiCommand::ResetAttributes]);
    }

    #[test]
    fn sgr_zero_param_resets_attributes() {
        assert_eq!(parse(b"\x1b[0m"), vec![AnsiCommand::ResetAttributes]);
    }

    #[test]
    fn sgr_sets_foreground_and_background() {
        assert_eq!(
            parse(b"\x1b[31;42m"),
            vec![
                AnsiCommand::SetForeground(AnsiColor::Red),
                AnsiCommand::SetBackground(AnsiColor::Green),
            ]
        );
    }

    #[test]
    fn sgr_sets_bright_colors() {
        assert_eq!(
            parse(b"\x1b[91;100m"),
            vec![
                AnsiCommand::SetForeground(AnsiColor::BrightRed),
                AnsiCommand::SetBackground(AnsiColor::BrightBlack),
            ]
        );
    }

    #[test]
    fn sgr_ignores_unsupported_params() {
        assert_eq!(
            parse(b"\x1b[1;35m"),
            vec![AnsiCommand::SetForeground(AnsiColor::Magenta)]
        );
    }

    #[test]
    fn cup_places_cursor_with_one_based_params() {
        assert_eq!(
            parse(b"\x1b[3;5H"),
            vec![AnsiCommand::SetCursorPos { row: 2, col: 4 }]
        );
    }

    #[test]
    fn cup_without_params_homes_the_cursor() {
        assert_eq!(
            parse(b"\x1b[H"),
            vec![AnsiCommand::SetCursorPos { row: 0, col: 0 }]
        );
    }

    #[test]
    fn cup_f_finalizer_places_cursor() {
        assert_eq!(
            parse(b"\x1b[2;3f"),
            vec![AnsiCommand::SetCursorPos { row: 1, col: 2 }]
        );
    }

    #[test]
    fn cup_zero_params_saturate_to_origin() {
        assert_eq!(
            parse(b"\x1b[0;0H"),
            vec![AnsiCommand::SetCursorPos { row: 0, col: 0 }]
        );
    }

    #[test]
    fn ed_clears_screen() {
        assert_eq!(parse(b"\x1b[2J"), vec![AnsiCommand::ClearScreen]);
    }

    #[test]
    fn el_clears_line() {
        assert_eq!(parse(b"\x1b[1K"), vec![AnsiCommand::ClearLine]);
    }

    #[test]
    fn unknown_csi_byte_aborts_sequence_without_command() {
        assert_eq!(
            parse(b"\x1b[?m"),
            vec![AnsiCommand::PrintChar(b'm')]
        );
    }

    #[test]
    fn printing_resumes_after_a_sequence() {
        assert_eq!(
            parse(b"\x1b[Jhi"),
            vec![
                AnsiCommand::ClearScreen,
                AnsiCommand::PrintChar(b'h'),
                AnsiCommand::PrintChar(b'i'),
            ]
        );
    }

    #[test]
    fn queue_keeps_fifteen_pending_commands_and_drops_the_rest() {
        let mut parser = AnsiParser::new();
        for _ in 0..20 {
            parser.handle_byte(b'A');
        }
        let mut kept = 0;
        while parser.next_command().is_some() {
            kept += 1;
        }
        assert_eq!(kept, 15);
    }
}
