#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    OuterDoc,
    InnerDoc,
    Plain,
    Block,
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub kind: CommentKind,
    pub start: usize,
    pub end: usize,
    pub first_line: usize,
    pub last_line: usize,
    pub text: String,
}

impl Comment {
    pub fn line_count(&self) -> usize {
        self.last_line - self.first_line + 1
    }
}

#[derive(Debug)]
pub struct Scanned {
    pub comments: Vec<Comment>,
    pub code: String,
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn blank(code: &mut [u8], start: usize, end: usize) {
    for byte in &mut code[start..end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

fn line_of(line_starts: &[usize], offset: usize) -> usize {
    line_starts.partition_point(|&start| start <= offset)
}

fn raw_string_hashes(bytes: &[u8], index: usize) -> Option<usize> {
    let starts_token = match index.checked_sub(1).map(|previous| bytes[previous]) {
        None => true,
        Some(previous) if !is_identifier_byte(previous) => true,
        Some(b'b' | b'c') => index == 1 || !is_identifier_byte(bytes[index - 2]),
        Some(_) => false,
    };
    if !starts_token {
        return None;
    }
    let mut cursor = index + 1;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b'"')).then_some(cursor - index - 1)
}

fn raw_string_end(bytes: &[u8], quote: usize, hashes: usize) -> usize {
    let mut cursor = quote + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"' && bytes[cursor + 1..].iter().take(hashes).filter(|&&b| b == b'#').count() == hashes {
            return cursor + 1 + hashes;
        }
        cursor += 1;
    }
    bytes.len()
}

fn string_end(bytes: &[u8], quote: usize) -> usize {
    let mut cursor = quote + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor += 2,
            b'"' => return cursor + 1,
            _ => cursor += 1,
        }
    }
    bytes.len()
}

fn char_literal_end(source: &str, quote: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let next = source[quote + 1..].chars().next()?;
    if next == '\\' {
        let mut cursor = quote + 3;
        while cursor < bytes.len() && bytes[cursor] != b'\'' {
            cursor += 1;
        }
        return Some((cursor + 1).min(bytes.len()));
    }
    let after = quote + 1 + next.len_utf8();
    (bytes.get(after) == Some(&b'\'')).then_some(after + 1)
}

fn block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut cursor = start;
    while cursor + 1 < bytes.len() {
        if bytes[cursor] == b'/' && bytes[cursor + 1] == b'*' {
            depth += 1;
            cursor += 2;
        } else if bytes[cursor] == b'*' && bytes[cursor + 1] == b'/' {
            depth -= 1;
            cursor += 2;
            if depth == 0 {
                return cursor;
            }
        } else {
            cursor += 1;
        }
    }
    bytes.len()
}

fn line_comment_kind(text: &str) -> CommentKind {
    if text.starts_with("///") && !text.starts_with("////") {
        CommentKind::OuterDoc
    } else if text.starts_with("//!") {
        CommentKind::InnerDoc
    } else {
        CommentKind::Plain
    }
}

pub fn scan(source: &str) -> Scanned {
    let bytes = source.as_bytes();
    let mut code = bytes.to_vec();
    let mut comments = Vec::new();
    let mut line_starts = vec![0usize];
    line_starts.extend(bytes.iter().enumerate().filter(|&(_, &b)| b == b'\n').map(|(i, _)| i + 1));
    let mut cursor = 0;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        let (end, kind) = if byte == b'/' && bytes.get(cursor + 1) == Some(&b'/') {
            let end = bytes[cursor..].iter().position(|&b| b == b'\n').map_or(bytes.len(), |p| cursor + p);
            (end, Some(line_comment_kind(&source[cursor..end])))
        } else if byte == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            (block_comment_end(bytes, cursor), Some(CommentKind::Block))
        } else if byte == b'"' {
            (string_end(bytes, cursor), None)
        } else if byte == b'r' && raw_string_hashes(bytes, cursor).is_some() {
            let hashes = raw_string_hashes(bytes, cursor).unwrap_or(0);
            (raw_string_end(bytes, cursor + 1 + hashes, hashes), None)
        } else if byte == b'\'' {
            match char_literal_end(source, cursor) {
                Some(end) => (end, None),
                None => {
                    cursor += 1;
                    continue;
                }
            }
        } else {
            cursor += 1;
            continue;
        };
        if let Some(kind) = kind {
            comments.push(Comment {
                kind,
                start: cursor,
                end,
                first_line: line_of(&line_starts, cursor),
                last_line: line_of(&line_starts, end.saturating_sub(1).max(cursor)),
                text: source[cursor..end].to_string(),
            });
        }
        blank(&mut code, cursor, end);
        cursor = end.max(cursor + 1);
    }
    Scanned {
        comments,
        code: String::from_utf8(code).expect("blanking preserves utf-8 boundaries"),
    }
}

pub fn count_words(code: &str, word: &str) -> usize {
    let bytes = code.as_bytes();
    let mut count = 0;
    let mut from = 0;
    while let Some(found) = code[from..].find(word) {
        let start = from + found;
        let end = start + word.len();
        let boundary_before = start == 0 || !is_identifier_byte(bytes[start - 1]);
        let boundary_after = end >= bytes.len() || !is_identifier_byte(bytes[end]);
        if boundary_before && boundary_after {
            count += 1;
        }
        from = end;
    }
    count
}

pub fn contains_asm_macro(code: &str) -> bool {
    ["asm", "global_asm", "naked_asm"]
        .iter()
        .any(|name| code.match_indices(name).any(|(start, _)| {
            let bytes = code.as_bytes();
            let end = start + name.len();
            (start == 0 || !is_identifier_byte(bytes[start - 1]))
                && code[end..].trim_start().starts_with('!')
                && !code[end..].trim_start().starts_with("!=")
        }))
}

fn matching_close(code: &str, open: usize) -> usize {
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    for (offset, &byte) in bytes[open..].iter().enumerate() {
        match byte {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return open + offset + 1;
                }
            }
            _ => {}
        }
    }
    bytes.len()
}

fn skip_whitespace(code: &str, mut position: usize) -> usize {
    let bytes = code.as_bytes();
    while position < bytes.len() && bytes[position].is_ascii_whitespace() {
        position += 1;
    }
    position
}

pub fn outer_item_extent(code: &str, after_comment: usize) -> (usize, usize) {
    let bytes = code.as_bytes();
    let mut position = skip_whitespace(code, after_comment);
    while code[position..].starts_with("#[") {
        position = skip_whitespace(code, matching_close(code, position + 1));
    }
    let start = position;
    let mut depth = 0usize;
    while position < bytes.len() {
        match bytes[position] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'{' if depth == 0 => return (start, matching_close(code, position)),
            b'{' => position = matching_close(code, position) - 1,
            b';' | b',' if depth == 0 => return (start, position + 1),
            b'}' if depth == 0 => return (start, position),
            _ => {}
        }
        position += 1;
    }
    (start, bytes.len())
}

pub fn inner_item_extent(code: &str, comment_start: usize) -> (usize, usize) {
    if code[..comment_start].trim().is_empty() {
        return (0, code.len());
    }
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    let mut open = None;
    for index in (0..comment_start).rev() {
        match bytes[index] {
            b'}' => depth += 1,
            b'{' if depth == 0 => {
                open = Some(index);
                break;
            }
            b'{' => depth -= 1,
            _ => {}
        }
    }
    match open {
        Some(index) => (index, matching_close(code, index)),
        None => (0, code.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<CommentKind> {
        scan(source).comments.iter().map(|c| c.kind).collect()
    }

    #[test]
    fn classifies_comment_forms() {
        let source = "// plain\n/// outer\n//! inner\n//// four\n/* block */\n/** doc block */\n";
        assert_eq!(
            kinds(source),
            vec![
                CommentKind::Plain,
                CommentKind::OuterDoc,
                CommentKind::InnerDoc,
                CommentKind::Plain,
                CommentKind::Block,
                CommentKind::Block
            ]
        );
    }

    #[test]
    fn slashes_inside_strings_are_not_comments() {
        let source = "let a = \"// no\"; let b = r#\"/* no \"# ; let c = b\"//\";\nlet d = '/'; let e = '\\'';\n";
        assert!(scan(source).comments.is_empty());
    }

    #[test]
    fn lifetimes_do_not_swallow_code() {
        let scanned = scan("fn f<'a>(x: &'a str) -> &'a str { x } // tail\n");
        assert_eq!(scanned.comments.len(), 1);
        assert!(scanned.code.contains("-> &'a str"));
        assert!(!scanned.code.contains("tail"));
    }

    #[test]
    fn nested_block_comments_end_at_the_outermost_close() {
        let scanned = scan("/* a /* b */ c */ let x = 1;");
        assert_eq!(scanned.comments.len(), 1);
        assert!(scanned.code.contains("let x = 1;"));
        assert!(!scanned.code.contains('b'));
    }

    #[test]
    fn line_numbers_are_one_based_and_span_blocks() {
        let scanned = scan("fn a() {}\n/* one\ntwo */\n// last\n");
        assert_eq!((scanned.comments[0].first_line, scanned.comments[0].last_line), (2, 3));
        assert_eq!((scanned.comments[1].first_line, scanned.comments[1].last_line), (4, 4));
    }

    #[test]
    fn words_respect_identifier_boundaries() {
        assert_eq!(count_words("unsafe { } not_unsafe unsafely unsafe fn", "unsafe"), 2);
        assert!(contains_asm_macro("asm!(\"nop\")"));
        assert!(contains_asm_macro("core::arch::global_asm!(\"x\")"));
        assert!(!contains_asm_macro("fn asmx() {} let y = bad_asm;"));
    }

    #[test]
    fn outer_extent_covers_attributes_and_body() {
        let code = "fn pre() {}\n#[inline]\nfn target() { unsafe { x() } }\nfn after() {}\n";
        let comment_end = code.find("#[inline]").unwrap();
        let (start, end) = outer_item_extent(code, comment_end);
        let item = &code[start..end];
        assert!(item.starts_with("fn target"));
        assert!(item.contains("unsafe"));
        assert!(!item.contains("after"));
    }

    #[test]
    fn outer_extent_of_a_field_stops_at_its_comma() {
        let code = "struct S {\n    a: u32,\n    b: u32,\n}\n";
        let (start, end) = outer_item_extent(code, code.find("a:").unwrap());
        assert_eq!(&code[start..end], "a: u32,");
    }

    #[test]
    fn outer_extent_of_a_semicolon_item_ends_there() {
        let code = "static X: [u8; 4] = [0; 4];\nfn after() { unsafe {} }\n";
        let (start, end) = outer_item_extent(code, 0);
        assert_eq!(&code[start..end], "static X: [u8; 4] = [0; 4];");
    }

    #[test]
    fn inner_extent_is_the_file_at_top_and_the_block_inside() {
        let code = "\nfn a() { unsafe {} }\nmod m {\n\n fn b() {}\n}\nfn c() {}\n";
        assert_eq!(inner_item_extent(code, 1), (0, code.len()));
        let inside = code.find("\n fn b").unwrap() - 1;
        let (start, end) = inner_item_extent(code, inside);
        assert!(code[start..end].starts_with("{\n\n fn b"));
        assert!(!code[start..end].contains("fn c"));
    }
}
