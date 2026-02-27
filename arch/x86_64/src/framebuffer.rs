use core::fmt;
use core::fmt::Write;
use core::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering::Relaxed};
use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use kernel::default_output::KernelOutput;
use lazy_static::lazy_static;
use spin::Mutex;
use crate::ansi_parser::{AnsiColor, AnsiCommand, AnsiParser};
use crate::terminal_fonts::{BitmapFont, IBM_8X8, IBM_VGA_8X16, TERMINUS_8X16, SPLEEN_8X16};

const FONT: &BitmapFont = &TERMINUS_8X16;

static FB_START:  AtomicU64   = AtomicU64::new(0);
static FB_WIDTH:  AtomicUsize = AtomicUsize::new(0);
static FB_HEIGHT: AtomicUsize = AtomicUsize::new(0);
static FB_STRIDE: AtomicUsize = AtomicUsize::new(0);
static FB_BPP:    AtomicUsize = AtomicUsize::new(4);
static FB_FMT:    AtomicU8    = AtomicU8::new(0); // 0 = RGB, 1 = BGR

pub fn init(buffer_start: u64, info: FrameBufferInfo) {
    FB_START.store(buffer_start, Relaxed);
    FB_WIDTH.store(info.width, Relaxed);
    FB_HEIGHT.store(info.height, Relaxed);
    FB_STRIDE.store(info.stride, Relaxed);
    FB_BPP.store(info.bytes_per_pixel, Relaxed);
    FB_FMT.store(match info.pixel_format {
        PixelFormat::Bgr => 1,
        _ => 0,
    }, Relaxed);
    unsafe {
        core::ptr::write_bytes(buffer_start as *mut u8, 0, info.stride * info.height * info.bytes_per_pixel);
    }
}

fn draw_char(col: usize, row: usize, ch: u8, fg: (u8, u8, u8), bg: (u8, u8, u8)) {
    let start = FB_START.load(Relaxed) as *mut u8;
    if start.is_null() { return; }
    let stride = FB_STRIDE.load(Relaxed);
    let bpp    = FB_BPP.load(Relaxed);
    let fmt    = FB_FMT.load(Relaxed);
    let glyph  = FONT.glyph(ch);
    let base_x = col * FONT.char_w;
    let base_y = row * FONT.char_h;
    for (bit_y, &row_bits) in glyph.iter().enumerate() {
        let row_base = (base_y + bit_y) * stride * bpp;
        for bit_x in 0..FONT.char_w {
            let bit = if FONT.lsb_first { (row_bits >> bit_x) & 1 } else { (row_bits >> (7 - bit_x)) & 1 };
            let (r, g, b) = if bit != 0 { fg } else { bg };
            let off = row_base + (base_x + bit_x) * bpp;
            unsafe {
                let ptr = start.add(off);
                if fmt == 1 { ptr.write(b); ptr.add(1).write(g); ptr.add(2).write(r); }
                else         { ptr.write(r); ptr.add(1).write(g); ptr.add(2).write(b); }
            }
        }
    }
}

fn scroll_up() {
    let start  = FB_START.load(Relaxed) as *mut u8;
    if start.is_null() { return; }
    let height = FB_HEIGHT.load(Relaxed);
    let stride = FB_STRIDE.load(Relaxed);
    let bpp    = FB_BPP.load(Relaxed);
    let row_bytes    = stride * bpp;
    let scroll_bytes = FONT.char_h * row_bytes;
    let total_bytes  = height * row_bytes;
    unsafe {
        core::ptr::copy(start.add(scroll_bytes), start, total_bytes - scroll_bytes);
        core::ptr::write_bytes(start.add(total_bytes - scroll_bytes), 0, scroll_bytes);
    }
}

fn ansi_to_rgb(c: AnsiColor) -> (u8, u8, u8) {
    match c {
        AnsiColor::Black         => (0,   0,   0),
        AnsiColor::Red           => (170, 0,   0),
        AnsiColor::Green         => (0,   170, 0),
        AnsiColor::Yellow        => (170, 170, 0),
        AnsiColor::Blue          => (0,   0,   170),
        AnsiColor::Magenta       => (170, 0,   170),
        AnsiColor::Cyan          => (0,   170, 170),
        AnsiColor::White         => (170, 170, 170),
        AnsiColor::BrightBlack   => (85,  85,  85),
        AnsiColor::BrightRed     => (255, 85,  85),
        AnsiColor::BrightGreen   => (85,  255, 85),
        AnsiColor::BrightYellow  => (255, 255, 85),
        AnsiColor::BrightBlue    => (85,  85,  255),
        AnsiColor::BrightMagenta => (255, 85,  255),
        AnsiColor::BrightCyan    => (85,  255, 255),
        AnsiColor::BrightWhite   => (255, 255, 255),
    }
}

struct Writer {
    col:        usize,
    row:        usize,
    text_cols:  usize,
    text_rows:  usize,
    fg:         (u8, u8, u8),
    bg:         (u8, u8, u8),
    default_fg: (u8, u8, u8),
    default_bg: (u8, u8, u8),
    ansi_parser: AnsiParser,
}

impl Writer {
    fn new() -> Self {
        let fg = (0, 255, 0);
        let bg = (0, 0, 0);
        Writer {
            col: 0,
            row: 0,
            text_cols:  FB_WIDTH.load(Relaxed)  / FONT.char_w,
            text_rows:  FB_HEIGHT.load(Relaxed) / FONT.char_h,
            fg,
            bg,
            default_fg: fg,
            default_bg: bg,
            ansi_parser: AnsiParser::new(),
        }
    }

    fn write_byte(&mut self, byte: u8) {
        self.ansi_parser.handle_byte(byte);
        while let Some(cmd) = self.ansi_parser.next_command() {
            match cmd {
                AnsiCommand::PrintChar(b)          => self.internal_write_byte(b),
                AnsiCommand::SetForeground(c)      => { self.fg = ansi_to_rgb(c); }
                AnsiCommand::SetBackground(c)      => { self.bg = ansi_to_rgb(c); }
                AnsiCommand::ResetAttributes       => { self.fg = self.default_fg; self.bg = self.default_bg; }
                AnsiCommand::SetCursorPos{row, col} => {
                    self.row = row.min(self.text_rows.saturating_sub(1));
                    self.col = col.min(self.text_cols.saturating_sub(1));
                }
                AnsiCommand::ClearScreen => {
                    let start = FB_START.load(Relaxed) as *mut u8;
                    if !start.is_null() {
                        let bytes = FB_HEIGHT.load(Relaxed) * FB_STRIDE.load(Relaxed) * FB_BPP.load(Relaxed);
                        unsafe { core::ptr::write_bytes(start, 0, bytes); }
                    }
                    self.row = 0;
                    self.col = 0;
                }
                AnsiCommand::ClearLine => {
                    for c in 0..self.text_cols { draw_char(c, self.row, b' ', self.fg, self.bg); }
                    self.col = 0;
                }
            }
        }
    }

    fn internal_write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            b'\t' => {
                let next_tab = (self.col / 8 + 1) * 8;
                self.col = next_tab.min(self.text_cols);
            }
            0x08 => {
                if self.col > 0 {
                    self.col -= 1;
                    draw_char(self.col, self.row, b' ', self.fg, self.bg);
                }
            }
            byte => {
                if self.col >= self.text_cols { self.new_line(); }
                draw_char(self.col, self.row, byte, self.fg, self.bg);
                self.col += 1;
            }
        }
    }

    fn new_line(&mut self) {
        self.col = 0;
        if self.text_rows > 0 && self.row + 1 < self.text_rows {
            self.row += 1;
        } else {
            scroll_up();
        }
    }

    fn write_string(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                0x20..=0x7E | b'\n' | b'\t' | 0x08 | 0x1B => self.write_byte(byte),
                _ => self.write_byte(b'?'),
            }
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

lazy_static! {
    static ref WRITER: Mutex<Writer> = Mutex::new(Writer::new());
}

pub struct FramebufferOutput;

impl KernelOutput for FramebufferOutput {
    fn write_str(&self, s: &str) {
        WRITER.lock().write_str(s).unwrap();
    }
}
