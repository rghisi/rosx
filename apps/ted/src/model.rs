use alloc::vec::Vec;

use crate::screen::TAB_STOP;

#[derive(Clone)]
pub struct Buffer {
    lines: Vec<Vec<u8>>,
    cursor_row: usize,
    cursor_col: usize,
    modified: bool,
}

impl Buffer {
    pub fn new() -> Buffer {
        Buffer {
            lines: alloc::vec![Vec::new()],
            cursor_row: 0,
            cursor_col: 0,
            modified: false,
        }
    }

    pub fn from_bytes(data: &[u8]) -> Buffer {
        if data.is_empty() {
            return Buffer::new();
        }
        let mut lines: Vec<Vec<u8>> = Vec::new();
        let mut current: Vec<u8> = Vec::new();
        for &byte in data {
            if byte == b'\n' {
                lines.push(core::mem::take(&mut current));
            } else {
                current.push(byte);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        Buffer {
            lines,
            cursor_row: 0,
            cursor_col: 0,
            modified: false,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        for line in &self.lines {
            out.extend_from_slice(line);
            out.push(b'\n');
        }
        out
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, row: usize) -> &[u8] {
        &self.lines[row]
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn mark_clean(&mut self) {
        self.modified = false;
    }

    pub fn insert_byte(&mut self, byte: u8) {
        let col = self.cursor_col.min(self.lines[self.cursor_row].len());
        self.lines[self.cursor_row].insert(col, byte);
        self.cursor_col = col + 1;
        self.modified = true;
    }

    pub fn insert_spaces_to_tab_stop(&mut self) {
        let width = crate::screen::display_width(self.line(self.cursor_row), self.cursor_col);
        let pad = TAB_STOP - (width % TAB_STOP);
        for _ in 0..pad {
            self.insert_byte(b' ');
        }
    }

    pub fn split_line(&mut self) {
        let rest = self.lines[self.cursor_row].split_off(self.cursor_col);
        self.lines.insert(self.cursor_row + 1, rest);
        self.cursor_row += 1;
        self.cursor_col = 0;
        self.modified = true;
    }

    pub fn backspace(&mut self) {
        self.modified = true;
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            self.lines[self.cursor_row].remove(self.cursor_col);
            return;
        }
        if self.cursor_row == 0 {
            self.modified = false;
            return;
        }
        let current = self.lines.remove(self.cursor_row);
        self.cursor_row -= 1;
        self.cursor_col = self.lines[self.cursor_row].len();
        self.lines[self.cursor_row].extend_from_slice(&current);
    }

    pub fn delete_char(&mut self) {
        if self.cursor_col < self.lines[self.cursor_row].len() {
            self.lines[self.cursor_row].remove(self.cursor_col);
            self.modified = true;
            return;
        }
        if self.cursor_row + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].extend_from_slice(&next);
            self.modified = true;
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
        }
        self.clamp_cursor();
    }

    pub fn move_right(&mut self) {
        if self.cursor_col < self.lines[self.cursor_row].len() {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
        self.clamp_cursor();
    }

    pub fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
        }
        self.clamp_cursor();
    }

    pub fn move_down(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
        }
        self.clamp_cursor();
    }

    pub fn line_start(&mut self) {
        self.cursor_col = 0;
    }

    pub fn line_end(&mut self) {
        self.cursor_col = self.lines[self.cursor_row].len();
    }

    fn clamp_cursor(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col > line_len {
            self.cursor_col = line_len;
        }
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn buffer(lines: &[&str]) -> Buffer {
        let mut b = Buffer::new();
        for (index, text) in lines.iter().enumerate() {
            if index > 0 {
                b.split_line();
            }
            for byte in text.as_bytes() {
                b.insert_byte(*byte);
            }
        }
        b
    }

    fn snapshot(b: &Buffer) -> Vec<String> {
        (0..b.line_count())
            .map(|row| String::from_utf8(b.line(row).to_vec()).unwrap())
            .collect()
    }

    #[test]
    fn new_buffer_is_one_empty_unmodified_line() {
        let b = Buffer::new();
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0), b"");
        assert_eq!(b.cursor(), (0, 0));
        assert!(!b.is_modified());
    }

    #[test]
    fn from_bytes_splits_on_newlines_without_trailing_phantom() {
        let b = Buffer::from_bytes(b"ab\ncd");
        assert_eq!(snapshot(&b), vec!["ab", "cd"]);
        let trailing = Buffer::from_bytes(b"ab\n");
        assert_eq!(snapshot(&trailing), vec!["ab"]);
        let empty = Buffer::from_bytes(b"");
        assert_eq!(snapshot(&empty), vec![""]);
    }

    #[test]
    fn to_bytes_terminates_every_line_with_lf() {
        let b = buffer(&["ab", "cd"]);
        assert_eq!(b.to_bytes(), b"ab\ncd\n");
    }

    #[test]
    fn round_trip_preserves_a_trailing_newline_file() {
        let text = b"hello\nworld\n";
        assert_eq!(Buffer::from_bytes(text).to_bytes(), text);
    }

    #[test]
    fn insert_advances_cursor_and_marks_modified() {
        let mut b = Buffer::new();
        b.insert_byte(b'x');
        b.insert_byte(b'y');
        assert_eq!(b.line(0), b"xy");
        assert_eq!(b.cursor(), (0, 2));
        assert!(b.is_modified());
    }

    #[test]
    fn insert_at_midline_splices() {
        let mut b = buffer(&["ac"]);
        b.line_start();
        b.move_right();
        b.insert_byte(b'b');
        assert_eq!(b.line(0), b"abc");
        assert_eq!(b.cursor(), (0, 2));
    }

    #[test]
    fn split_moves_remainder_to_a_new_line() {
        let mut b = buffer(&["abcd"]);
        b.cursor_col = 2;
        b.split_line();
        assert_eq!(snapshot(&b), vec!["ab", "cd"]);
        assert_eq!(b.cursor(), (1, 0));
    }

    #[test]
    fn backspace_deletes_before_cursor() {
        let mut b = buffer(&["abc"]);
        b.line_end();
        b.backspace();
        assert_eq!(b.line(0), b"ab");
        assert_eq!(b.cursor(), (0, 2));
    }

    #[test]
    fn backspace_at_column_zero_joins_previous_line() {
        let mut b = Buffer::from_bytes(b"ab\ncd\n");
        b.move_down();
        b.backspace();
        assert_eq!(snapshot(&b), vec!["abcd"]);
        assert_eq!(b.cursor(), (0, 2));
    }

    #[test]
    fn backspace_at_origin_is_inert_and_unmodified() {
        let mut b = Buffer::new();
        b.backspace();
        assert_eq!(snapshot(&b), vec![""]);
        assert!(!b.is_modified());
    }

    #[test]
    fn delete_char_removes_under_cursor_or_joins_next_line() {
        let mut b = Buffer::from_bytes(b"abc");
        b.delete_char();
        assert_eq!(b.line(0), b"bc");
        let mut joined = Buffer::from_bytes(b"ab\ncd\n");
        joined.line_end();
        joined.delete_char();
        assert_eq!(snapshot(&joined), vec!["abcd"]);
        assert_eq!(joined.cursor(), (0, 2));
    }

    #[test]
    fn delete_at_last_cell_is_inert() {
        let mut b = Buffer::from_bytes(b"ab\n");
        b.line_end();
        b.delete_char();
        assert_eq!(b.line(0), b"ab");
        assert!(!b.is_modified());
    }

    #[test]
    fn move_right_crosses_line_boundary() {
        let mut b = Buffer::from_bytes(b"ab\ncd\n");
        b.line_end();
        b.move_right();
        assert_eq!(b.cursor(), (1, 0));
    }

    #[test]
    fn move_left_crosses_back_over_boundary() {
        let mut b = Buffer::from_bytes(b"ab\ncd\n");
        b.move_down();
        b.move_left();
        assert_eq!(b.cursor(), (0, 2));
    }

    #[test]
    fn vertical_moves_clamp_column() {
        let mut b = Buffer::from_bytes(b"longer line\nx\n");
        b.cursor_col = 10;
        b.move_down();
        assert_eq!(b.cursor(), (1, 1));
        b.move_up();
        assert_eq!(b.cursor(), (0, 1));
    }

    #[test]
    fn edge_moves_stay_in_bounds() {
        let mut b = buffer(&["a"]);
        b.move_left();
        assert_eq!(b.cursor(), (0, 0));
        b.line_end();
        b.move_right();
        assert_eq!(b.cursor(), (0, 1));
        b.move_up();
        assert_eq!(b.cursor(), (0, 1));
        b.move_down();
        assert_eq!(b.cursor(), (0, 1));
    }

    #[test]
    fn tab_stop_inserts_spaces_to_next_multiple_of_four() {
        let mut b = Buffer::new();
        b.insert_spaces_to_tab_stop();
        assert_eq!(b.line(0), b"    ");
        assert_eq!(b.cursor(), (0, 4));
        b.insert_byte(b'a');
        b.insert_spaces_to_tab_stop();
        assert_eq!(b.line(0), b"    a   ");
        assert_eq!(b.cursor(), (0, 8));
    }

    #[test]
    fn cursor_col_never_exceeds_line_length() {
        let mut b = buffer(&["abc", "z"]);
        b.line_end();
        b.move_down();
        assert!(b.cursor().1 <= b.line(b.cursor().0).len());
    }
}
