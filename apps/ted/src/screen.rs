use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use core::fmt::Write as _;

pub const COLS: usize = 80;
pub const TEXT_ROWS: usize = 24;
pub const STATUS_ROW: usize = TEXT_ROWS;
pub const TOTAL_ROWS: usize = TEXT_ROWS + 1;
pub const TAB_STOP: usize = 4;

pub fn display_width(line: &[u8], col: usize) -> usize {
    let mut width = 0usize;
    for &byte in &line[..col.min(line.len())] {
        if byte == b'\t' {
            width += TAB_STOP - width % TAB_STOP;
        } else {
            width += 1;
        }
    }
    width
}

pub fn row_cells(line: &[u8], left: usize, width: usize) -> Vec<u8> {
    let mut cells: Vec<u8> = Vec::with_capacity(left + width);
    for &byte in line {
        if cells.len() >= left + width {
            break;
        }
        if byte == b'\t' {
            let pad = TAB_STOP - cells.len() % TAB_STOP;
            for _ in 0..pad {
                cells.push(b' ');
            }
        } else {
            cells.push(byte);
        }
    }
    let mut window = if cells.len() <= left {
        Vec::new()
    } else {
        cells[left..].to_vec()
    };
    window.resize(width, b' ');
    window
}

#[derive(Default, Clone, Copy)]
pub struct View {
    pub top_row: usize,
    pub left: usize,
}

impl View {
    pub fn follow(&mut self, cursor_row: usize, cursor_display: usize) {
        if cursor_row < self.top_row {
            self.top_row = cursor_row;
        }
        if cursor_row >= self.top_row + TEXT_ROWS {
            self.top_row = cursor_row + 1 - TEXT_ROWS;
        }
        if cursor_display < self.left {
            self.left = cursor_display;
        }
        if cursor_display >= self.left + COLS {
            self.left = cursor_display + 1 - COLS;
        }
    }
}

pub fn cursor_display_offset(line: &[u8], left: usize, col: usize) -> Option<usize> {
    let display = display_width(line, col);
    if display < left || display >= left + COLS {
        None
    } else {
        Some(display - left)
    }
}

pub fn status_line(path: Option<&str>, modified: bool, row: usize, col_display: usize) -> String {
    let name: String = match path {
        Some(p) => {
            let characters: Vec<char> = p.chars().collect();
            if characters.len() > 24 {
                characters[characters.len() - 24..].iter().collect()
            } else {
                String::from(p)
            }
        }
        None => String::from("[no name]"),
    };
    let flag = if modified { "*" } else { "" };
    format!(
        "\x1B[100m\x1B[97m ted {name:<24}{flag:<1} Ln {}, Col {}   ^S Save  ^Q Quit\x1B[0m",
        row + 1,
        col_display + 1
    )
}

pub fn notice_line(message: &str) -> String {
    format!("\x1B[100m\x1B[93m {message}\x1B[0m")
}

pub struct Screen {
    plain: Vec<Vec<u8>>,
    status: String,
    cursor: Option<(usize, usize)>,
}

impl Screen {
    pub fn new() -> Screen {
        Screen {
            plain: alloc::vec![Vec::new(); TOTAL_ROWS],
            status: String::new(),
            cursor: None,
        }
    }

    pub fn repaint(&mut self, rows: &[Vec<u8>], status: &str, cursor: Option<(usize, usize)>) {
        let mut frame = String::new();
        let cursor_moved = self.cursor != cursor;
        for row in 0..TEXT_ROWS {
            let content = rows.get(row).cloned().unwrap_or_default();
            let cursor_touched = cursor_moved
                && (self.cursor.map(|(r, _)| r) == Some(row) || cursor.map(|(r, _)| r) == Some(row));
            if self.plain[row] != content || cursor_touched {
                let _ = write!(frame, "\x1B[{};1H\x1B[K", row + 1);
                frame.push_str(&String::from_utf8_lossy(&content));
            }
            self.plain[row] = content;
        }
        if self.status != status {
            let _ = write!(frame, "\x1B[{};1H\x1B[K", STATUS_ROW + 1);
            frame.push_str(status);
            self.status = String::from(status);
        }
        if let Some((row, col)) = cursor {
            let cell = self
                .plain
                .get(row)
                .and_then(|r| r.get(col))
                .copied()
                .unwrap_or(b' ');
            let _ = write!(
                frame,
                "\x1B[{};{}H\x1B[47m\x1B[30m{}\x1B[0m",
                row + 1,
                col + 1,
                cell as char
            );
        }
        if !frame.is_empty() {
            usrlib::print!("{}", frame);
        }
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_counts_tab_cells() {
        assert_eq!(display_width(b"abc", 3), 3);
        assert_eq!(display_width(b"\t", 1), 4);
        assert_eq!(display_width(b"a\tb", 3), 5);
        assert_eq!(display_width(b"abc\t", 4), 4);
        assert_eq!(display_width(b"x\ty", 2), 4);
    }

    #[test]
    fn row_cells_expands_tabs_pads_and_clips_to_the_window() {
        assert_eq!(row_cells(b"ab", 0, 5), *b"ab   ");
        assert_eq!(row_cells(b"a\tb", 0, 8), *b"a   b   ");
        assert_eq!(row_cells(b"abcdef", 2, 3), *b"cde");
        assert_eq!(row_cells(b"", 0, 4), *b"    ");
        assert_eq!(row_cells(b"short", 10, 4), *b"    ");
    }

    #[test]
    fn row_cells_tab_never_straddles_the_window_start() {
        assert_eq!(row_cells(b"\tx", 2, 4), *b"  x ");
    }

    #[test]
    fn view_follows_cursor_vertically() {
        let mut view = View::default();
        view.follow(TEXT_ROWS, 0);
        assert_eq!(view.top_row, 1);
        view.follow(0, 0);
        assert_eq!(view.top_row, 0);
    }

    #[test]
    fn view_follows_cursor_horizontally() {
        let mut view = View::default();
        view.follow(0, COLS);
        assert_eq!(view.left, 1);
        view.follow(0, 0);
        assert_eq!(view.left, 0);
    }

    #[test]
    fn view_keeps_the_window_when_the_cursor_is_visible() {
        let mut view = View { top_row: 10, left: 5 };
        view.follow(12, 8);
        assert_eq!(view.top_row, 10);
        assert_eq!(view.left, 5);
    }

    #[test]
    fn cursor_display_offset_is_none_outside_the_window() {
        assert_eq!(cursor_display_offset(b"abcdef", 0, 3), Some(3));
        assert_eq!(cursor_display_offset(b"abcdef", 4, 3), None);
        assert_eq!(cursor_display_offset(b"a\tb", 0, 3), Some(5));
    }

    #[test]
    fn status_line_shows_name_dirty_flag_and_position() {
        let bar = status_line(Some("/hello.txt"), true, 9, 12);
        assert!(bar.contains("/hello.txt"));
        assert!(bar.contains('*'));
        assert!(bar.contains("Ln 10, Col 13"));
        assert!(bar.contains("^S Save"));
        assert!(bar.contains("^Q Quit"));
    }

    #[test]
    fn status_line_without_a_path_marks_no_name_and_clean() {
        let bar = status_line(None, false, 0, 0);
        assert!(bar.contains("[no name]"));
        assert!(!bar.contains('*'));
    }

    #[test]
    fn long_paths_are_tail_kept_not_overflowed() {
        let bar = status_line(Some("/very/deeply/nested/directory/file.txt"), false, 0, 0);
        assert!(bar.contains("file.txt"));
        assert!(!bar.contains("/very/deeply/nested/directory/file.txt"));
    }

    #[test]
    fn notice_carries_its_message() {
        assert!(notice_line("saved 13 bytes").contains("saved 13 bytes"));
    }
}
