use core::fmt;
use core::fmt::Write;
use core::sync::atomic::{AtomicU64, Ordering};
use kernel::default_output::KernelOutput;
use lazy_static::lazy_static;
use spin::Mutex;
use volatile::Volatile;
use crate::ansi_parser::{AnsiParser, AnsiCommand, AnsiColor};

static VGA_PHYS_OFFSET: AtomicU64 = AtomicU64::new(0);

pub fn init(physical_memory_offset: u64) {
    VGA_PHYS_OFFSET.store(physical_memory_offset, Ordering::Relaxed);
}

lazy_static! {
    static ref WRITER: Mutex<Writer> =
        Mutex::new(Writer::new(ColorCode::new(Color::Green, Color::Black)));
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ColorCode(u8);
impl ColorCode {
    pub fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode((background as u8) << 4 | (foreground as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

#[repr(transparent)]
struct Buffer {
    chars: [[Volatile<ScreenChar>; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

struct Writer {
    column_position: usize,
    row_position: usize,
    color_code: ColorCode,
    default_color: ColorCode,
    buffer: &'static mut Buffer,
    ansi_parser: AnsiParser,
}

impl Writer {
    pub fn new(color_code: ColorCode) -> Writer {
        Writer {
            column_position: 0,
            row_position: BUFFER_HEIGHT - 1,
            color_code,
            default_color: color_code,
            buffer: unsafe { &mut *((VGA_PHYS_OFFSET.load(Ordering::Relaxed) + 0xb8000) as *mut Buffer) },
            ansi_parser: AnsiParser::new(),
        }
    }

    pub fn write_byte(&mut self, byte: u8) {
        self.ansi_parser.handle_byte(byte);
        while let Some(command) = self.ansi_parser.next_command() {
            match command {
                AnsiCommand::PrintChar(b) => self.internal_write_byte(b),
                AnsiCommand::SetForeground(fg) => {
                    let bg = Color::from_u8(self.color_code.0 >> 4);
                    self.color_code = ColorCode::new(Color::from_ansi(fg), bg);
                }
                AnsiCommand::SetBackground(bg) => {
                    let fg = Color::from_u8(self.color_code.0 & 0x0F);
                    self.color_code = ColorCode::new(fg, Color::from_ansi(bg));
                }
                AnsiCommand::ResetAttributes => {
                    self.color_code = self.default_color;
                }
                AnsiCommand::SetCursorPos { row, col } => {
                    self.row_position = row.min(BUFFER_HEIGHT - 1);
                    self.column_position = col.min(BUFFER_WIDTH - 1);
                }
                AnsiCommand::ClearScreen => {
                    for row in 0..BUFFER_HEIGHT {
                        self.clear_row(row);
                    }
                    self.row_position = 0;
                    self.column_position = 0;
                }
                AnsiCommand::ClearLine => {
                    self.clear_row(self.row_position);
                }
            }
        }
    }

    fn internal_write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            b'\t' => {
                let next_tab_stop = (self.column_position / 8 + 1) * 8;
                self.column_position = next_tab_stop.min(BUFFER_WIDTH);
            }
            0x08 => {
                if self.column_position > 0 {
                    self.column_position -= 1;
                    let row = self.row_position;
                    let col = self.column_position;
                    let color_code = self.color_code;
                    self.buffer.chars[row][col].write(ScreenChar {
                        ascii_character: b' ',
                        color_code,
                    });
                }
            }
            byte => {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }

                let row = self.row_position;
                let col = self.column_position;

                let color_code = self.color_code;
                self.buffer.chars[row][col].write(ScreenChar {
                    ascii_character: byte,
                    color_code,
                });
                self.column_position += 1;
            }
        }
    }

    fn new_line(&mut self) {
        if self.row_position < BUFFER_HEIGHT - 1 {
            self.row_position += 1;
        } else {
            for row in 1..BUFFER_HEIGHT {
                for col in 0..BUFFER_WIDTH {
                    let character = self.buffer.chars[row][col].read();
                    self.buffer.chars[row - 1][col].write(character);
                }
            }
            self.clear_row(BUFFER_HEIGHT - 1);
        }
        self.column_position = 0;
    }

    fn clear_row(&mut self, row: usize) {
        let blank = ScreenChar {
            ascii_character: b' ',
            color_code: self.color_code,
        };
        for col in 0..BUFFER_WIDTH {
            self.buffer.chars[row][col].write(blank);
        }
    }

    pub fn write_string(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                // printable ASCII byte, newline, backspace or escape
                0x20..=0x7e | b'\n' | b'\t' | 0x08 | 0x1B => self.write_byte(byte),
                // not part of printable ASCII range
                _ => self.write_byte(0xfe),
            }
        }
    }
}

impl Color {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Color::Black,
            1 => Color::Blue,
            2 => Color::Green,
            3 => Color::Cyan,
            4 => Color::Red,
            5 => Color::Magenta,
            6 => Color::Brown,
            7 => Color::LightGray,
            8 => Color::DarkGray,
            9 => Color::LightBlue,
            10 => Color::LightGreen,
            11 => Color::LightCyan,
            12 => Color::LightRed,
            13 => Color::Pink,
            14 => Color::Yellow,
            15 => Color::White,
            _ => Color::White,
        }
    }

    fn from_ansi(ansi_color: AnsiColor) -> Self {
        match ansi_color {
            AnsiColor::Black => Color::Black,
            AnsiColor::Red => Color::Red,
            AnsiColor::Green => Color::Green,
            AnsiColor::Yellow => Color::Brown,
            AnsiColor::Blue => Color::Blue,
            AnsiColor::Magenta => Color::Magenta,
            AnsiColor::Cyan => Color::Cyan,
            AnsiColor::White => Color::LightGray,
            AnsiColor::BrightBlack => Color::DarkGray,
            AnsiColor::BrightRed => Color::LightRed,
            AnsiColor::BrightGreen => Color::LightGreen,
            AnsiColor::BrightYellow => Color::Yellow,
            AnsiColor::BrightBlue => Color::LightBlue,
            AnsiColor::BrightMagenta => Color::Pink,
            AnsiColor::BrightCyan => Color::LightCyan,
            AnsiColor::BrightWhite => Color::White,
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

pub struct VgaOutput;

impl KernelOutput for VgaOutput {
    fn write_str(&self, s: &str) {
        WRITER.lock().write_str(s).unwrap();
    }
}
