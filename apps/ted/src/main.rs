#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use usrlib::print;
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::client::Vfs;

use ted::file::{self, LoadOutcome};
use ted::keys::{self, Command};
use ted::model::Buffer;
use ted::screen::{self, Screen, View, COLS, TEXT_ROWS};

enum Mode {
    Edit,
    QuitPrompt,
}

struct Editor {
    buffer: Buffer,
    view: View,
    screen: Screen,
    mode: Mode,
    notice: Option<String>,
    path: Option<&'static str>,
    on_disk: Option<usize>,
}

impl Editor {
    fn render(&mut self) {
        let (row, col) = self.buffer.cursor();
        let line = self.buffer.line(row);
        let display = screen::display_width(line, col);
        self.view.follow(row, display);
        let mut rows: Vec<Vec<u8>> = Vec::with_capacity(TEXT_ROWS);
        for window_row in 0..TEXT_ROWS {
            let model_row = self.view.top_row + window_row;
            let source = if model_row < self.buffer.line_count() {
                self.buffer.line(model_row)
            } else {
                b""
            };
            rows.push(screen::row_cells(source, self.view.left, COLS));
        }
        let status = match &self.notice {
            Some(message) => screen::notice_line(message),
            None => match self.mode {
                Mode::QuitPrompt => screen::notice_line("Save modified buffer? (Y/N)"),
                Mode::Edit => screen::status_line(
                    self.path,
                    self.buffer.is_modified(),
                    row,
                    display - self.view.left,
                ),
            },
        };
        let cursor_col = screen::cursor_display_offset(line, self.view.left, col)
            .unwrap_or(COLS - 1);
        self.screen.repaint(&rows, &status, Some((row, cursor_col)));
    }

    fn save_buffer(&mut self) -> Result<(), String> {
        let Some(path) = self.path else {
            return Err(String::from("^S needs a file name: run `ted <path>`"));
        };
        let vfs = Vfs::connect().map_err(|error| format!("no filesystem service: {error:?}"))?;
        let bytes = self.buffer.to_bytes();
        let outcome = file::save(&vfs, path, &bytes, self.on_disk);
        vfs.disconnect();
        outcome.map(|()| {
            self.on_disk = Some(bytes.len());
            self.buffer.mark_clean();
            self.notice = Some(format!("saved {} bytes", bytes.len()));
        })
    }

    fn handle(&mut self, c: char) -> bool {
        self.notice = None;
        if matches!(self.mode, Mode::QuitPrompt) {
            return match c {
                'y' | 'Y' => match self.save_buffer() {
                    Ok(()) => true,
                    Err(message) => {
                        self.notice = Some(message);
                        false
                    }
                },
                'n' | 'N' | '\x1b' => {
                    self.mode = Mode::Edit;
                    false
                }
                _ => false,
            };
        }
        match keys::command_for(c) {
            Some(Command::Insert(byte)) => self.buffer.insert_byte(byte as u8),
            Some(Command::Enter) => self.buffer.split_line(),
            Some(Command::Tab) => self.buffer.insert_spaces_to_tab_stop(),
            Some(Command::Backspace) => self.buffer.backspace(),
            Some(Command::Delete) => self.buffer.delete_char(),
            Some(Command::Left) => self.buffer.move_left(),
            Some(Command::Right) => self.buffer.move_right(),
            Some(Command::Up) => self.buffer.move_up(),
            Some(Command::Down) => self.buffer.move_down(),
            Some(Command::LineStart) => self.buffer.line_start(),
            Some(Command::LineEnd) => self.buffer.line_end(),
            Some(Command::Save) => {
                if let Err(message) = self.save_buffer() {
                    self.notice = Some(message);
                }
            }
            Some(Command::Quit) => {
                if self.buffer.is_modified() {
                    self.mode = Mode::QuitPrompt;
                } else {
                    return true;
                }
            }
            None => {}
        }
        false
    }
}

fn main() {
    let arguments = usrlib::args();
    let path: Option<&'static str> = arguments.get(1).copied();
    let vfs = match Vfs::connect() {
        Ok(vfs) => vfs,
        Err(error) => {
            println!("ted: no filesystem service ({error:?})");
            Syscall::exit(1);
        }
    };
    let (buffer, on_disk) = match path {
        Some(path) => match file::load(&vfs, path) {
            Ok(LoadOutcome::Exists(bytes)) => (Buffer::from_bytes(&bytes), Some(bytes.len())),
            Ok(LoadOutcome::Absent) => (Buffer::new(), None),
            Err(message) => {
                println!("ted: {message}");
                Syscall::exit(1);
            }
        },
        None => (Buffer::new(), None),
    };
    vfs.disconnect();
    let mut editor = Editor {
        buffer,
        view: View::default(),
        screen: Screen::new(),
        mode: Mode::Edit,
        notice: None,
        path,
        on_disk,
    };
    print!("\x1B[2J\x1B[H");
    loop {
        editor.render();
        let key = Syscall::read_char();
        if editor.handle(key) {
            break;
        }
    }
    print!("\x1B[0m\x1B[2J\x1B[H");
    Syscall::exit(0);
}
