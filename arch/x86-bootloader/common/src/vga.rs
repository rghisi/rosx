use crate::debugport::render_hex;
use crate::pio;

const VGA_TEXT_SCREEN_ADDRESS: usize = 0xB8000;
const CURSOR_ROW_ADDRESS: usize = 0x0450;
const CURSOR_COLUMN_ADDRESS: usize = 0x0451;
const CRTC_INDEX_PORT: u16 = 0x3D4;
const CRTC_DATA_PORT: u16 = 0x3D5;
const CRTC_CURSOR_OFFSET_HIGH: u8 = 0x0E;
const CRTC_CURSOR_OFFSET_LOW: u8 = 0x0F;

pub const COLUMNS: usize = 80;
pub const ROWS: usize = 25;
const CELLS: usize = COLUMNS * ROWS;
const BRIGHT_WHITE_ON_BLACK: u16 = 0x0F00;
const LARGEST_HEX_BYTES: usize = 18;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TtyCursor {
    pub row: usize,
    pub column: usize,
}

pub fn append_at(screen: &mut [u16], start: TtyCursor, text: &str) -> TtyCursor {
    let mut cursor = start;
    for glyph in text.bytes() {
        match glyph {
            b'\r' => cursor.column = 0,
            b'\n' => cursor.row = (cursor.row + 1).min(ROWS - 1),
            printable => {
                if cursor.column == COLUMNS {
                    cursor.column = 0;
                    cursor.row = (cursor.row + 1).min(ROWS - 1);
                }
                if let Some(cell) = screen.get_mut(cursor.row * COLUMNS + cursor.column) {
                    *cell = u16::from(printable) | BRIGHT_WHITE_ON_BLACK;
                }
                cursor.column += 1;
            }
        }
    }
    cursor
}

pub fn append(text: &str) {
    let cursor = append_at(screen_window(), crtc_cursor(), text);
    publish_cursor(cursor);
}

pub fn append_hex(value: u64, nibbles: u32) {
    let mut buffer = [0u8; LARGEST_HEX_BYTES];
    let mut length = 0usize;
    render_hex(|byte| {
        if length < buffer.len() {
            buffer[length] = byte;
            length += 1;
        }
    }, value, nibbles);
    if let Ok(rendered) = core::str::from_utf8(&buffer[..length]) {
        append(rendered);
    }
}

fn screen_window() -> &'static mut [u16] {
    unsafe { core::slice::from_raw_parts_mut(VGA_TEXT_SCREEN_ADDRESS as *mut u16, CELLS) }
}

fn crtc_cursor() -> TtyCursor {
    let offset = read_crtc_register(CRTC_CURSOR_OFFSET_HIGH) * 0x100
        + read_crtc_register(CRTC_CURSOR_OFFSET_LOW);
    TtyCursor {
        row: (offset / COLUMNS).min(ROWS - 1),
        column: offset % COLUMNS,
    }
}

fn read_crtc_register(register: u8) -> usize {
    pio::outb(CRTC_INDEX_PORT, register);
    usize::from(pio::inb(CRTC_DATA_PORT))
}

fn publish_cursor(cursor: TtyCursor) {
    let offset = cursor.row * COLUMNS + cursor.column;
    write_byte(CURSOR_ROW_ADDRESS, cursor.row as u8);
    write_byte(CURSOR_COLUMN_ADDRESS, cursor.column as u8);
    pio::outb(CRTC_INDEX_PORT, CRTC_CURSOR_OFFSET_HIGH);
    pio::outb(CRTC_DATA_PORT, (offset >> 8) as u8);
    pio::outb(CRTC_INDEX_PORT, CRTC_CURSOR_OFFSET_LOW);
    pio::outb(CRTC_DATA_PORT, offset as u8);
}

fn write_byte(address: usize, value: u8) {
    unsafe { core::ptr::write_volatile(address as *mut u8, value) }
}

#[cfg(test)]
mod tests {
    use super::{append_at, CELLS, COLUMNS, ROWS, TtyCursor};

    fn blank_screen() -> Vec<u16> {
        vec![0u16; CELLS]
    }

    fn cell_at(screen: &[u16], row: usize, column: usize) -> u16 {
        screen[row * COLUMNS + column]
    }

    fn origin() -> TtyCursor {
        TtyCursor { row: 0, column: 0 }
    }

    #[test]
    fn writes_the_glyph_with_the_bright_white_attribute() {
        let mut screen = blank_screen();
        let cursor = append_at(&mut screen, origin(), "A");
        assert_eq!(cell_at(&screen, 0, 0), u16::from(b'A') | 0x0F00);
        assert_eq!(cursor, TtyCursor { row: 0, column: 1 });
    }

    #[test]
    fn carriage_return_resets_the_column_only() {
        let mut screen = blank_screen();
        let cursor = append_at(&mut screen, TtyCursor { row: 3, column: 40 }, "abc\rxy");
        assert_eq!(cell_at(&screen, 3, 0), u16::from(b'x') | 0x0F00);
        assert_eq!(cell_at(&screen, 3, 1), u16::from(b'y') | 0x0F00);
        assert_eq!(cursor, TtyCursor { row: 3, column: 2 });
    }

    #[test]
    fn line_feed_advances_the_row_and_clamps_at_the_bottom() {
        let mut screen = blank_screen();
        let cursor = append_at(
            &mut screen,
            TtyCursor {
                row: ROWS - 1,
                column: 10,
            },
            "\n\n\n",
        );
        assert_eq!(cursor.row, ROWS - 1);
    }

    #[test]
    fn text_wraps_at_the_column_limit() {
        let mut screen = blank_screen();
        let text = "x".repeat(COLUMNS + 3);
        let cursor = append_at(&mut screen, origin(), &text);
        assert_eq!(cell_at(&screen, 0, COLUMNS - 1), u16::from(b'x') | 0x0F00);
        assert_eq!(cell_at(&screen, 1, 2), u16::from(b'x') | 0x0F00);
        assert_eq!(cursor, TtyCursor { row: 1, column: 3 });
    }

    #[test]
    fn the_bottom_row_clamps_by_rewriting_not_scrolling() {
        let mut screen = blank_screen();
        let text = "y".repeat(COLUMNS + 3);
        let cursor = append_at(
            &mut screen,
            TtyCursor {
                row: ROWS - 1,
                column: COLUMNS - 1,
            },
            &text,
        );
        assert_eq!(cell_at(&screen, ROWS - 1, 2), u16::from(b'y') | 0x0F00);
        assert_eq!(cell_at(&screen, ROWS - 1, COLUMNS - 1), u16::from(b'y') | 0x0F00);
        assert_eq!(cursor, TtyCursor { row: ROWS - 1, column: 2 });
    }

    #[test]
    fn a_short_screen_drops_cells_it_cannot_hold() {
        let mut screen = vec![0u16; 4];
        let cursor = append_at(&mut screen, origin(), "hello");
        assert_eq!(screen, vec![u16::from(b'h') | 0x0F00, u16::from(b'e') | 0x0F00, u16::from(b'l') | 0x0F00, u16::from(b'l') | 0x0F00]);
        assert_eq!(cursor, TtyCursor { row: 0, column: 5 });
    }
}
