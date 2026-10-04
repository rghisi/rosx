const LOWERCASE_TOKENS: [&str; 26] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];

const SHIFTED_TOKENS: [&str; 26] = [
    "shift-a",
    "shift-b",
    "shift-c",
    "shift-d",
    "shift-e",
    "shift-f",
    "shift-g",
    "shift-h",
    "shift-i",
    "shift-j",
    "shift-k",
    "shift-l",
    "shift-m",
    "shift-n",
    "shift-o",
    "shift-p",
    "shift-q",
    "shift-r",
    "shift-s",
    "shift-t",
    "shift-u",
    "shift-v",
    "shift-w",
    "shift-x",
    "shift-y",
    "shift-z",
];

const DIGIT_TOKENS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

pub fn token_for(c: char) -> Option<&'static str> {
    if c.is_ascii_lowercase() {
        return Some(LOWERCASE_TOKENS[c as usize - 'a' as usize]);
    }
    if c.is_ascii_uppercase() {
        return Some(SHIFTED_TOKENS[c as usize - 'A' as usize]);
    }
    if c.is_ascii_digit() {
        return Some(DIGIT_TOKENS[c as usize - '0' as usize]);
    }
    match c {
        '\n' => Some("ret"),
        ' ' => Some("sp"),
        '\t' => Some("tab"),
        '\x08' => Some("backspace"),
        '-' => Some("minus"),
        '=' => Some("equal"),
        '.' => Some("dot"),
        ',' => Some("comma"),
        '/' => Some("slash"),
        ';' => Some("semicolon"),
        '\'' => Some("apostrophe"),
        '`' => Some("grave"),
        '\\' => Some("backslash"),
        '[' => Some("bracket_left"),
        ']' => Some("bracket_right"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::token_for;

    #[test]
    fn lowercase_letters_map_to_themselves() {
        for c in 'a'..='z' {
            assert_eq!(token_for(c).map(str::to_string), Some(c.to_string()));
        }
    }

    #[test]
    fn uppercase_letters_map_to_shifted_lowercase() {
        for c in 'A'..='Z' {
            let lower = c.to_ascii_lowercase();
            assert_eq!(token_for(c).map(str::to_string), Some(format!("shift-{lower}")));
        }
    }

    #[test]
    fn digits_map_to_themselves() {
        for c in '0'..='9' {
            assert_eq!(token_for(c).map(str::to_string), Some(c.to_string()));
        }
    }

    #[test]
    fn special_characters_map_to_named_tokens() {
        assert_eq!(token_for('\n'), Some("ret"));
        assert_eq!(token_for(' '), Some("sp"));
        assert_eq!(token_for('\t'), Some("tab"));
        assert_eq!(token_for('\x08'), Some("backspace"));
        assert_eq!(token_for('-'), Some("minus"));
        assert_eq!(token_for('='), Some("equal"));
        assert_eq!(token_for('.'), Some("dot"));
        assert_eq!(token_for(','), Some("comma"));
        assert_eq!(token_for('/'), Some("slash"));
        assert_eq!(token_for(';'), Some("semicolon"));
        assert_eq!(token_for('\''), Some("apostrophe"));
        assert_eq!(token_for('`'), Some("grave"));
        assert_eq!(token_for('\\'), Some("backslash"));
        assert_eq!(token_for('['), Some("bracket_left"));
        assert_eq!(token_for(']'), Some("bracket_right"));
    }

    #[test]
    fn unsupported_characters_return_none() {
        assert_eq!(token_for('!'), None);
        assert_eq!(token_for('#'), None);
        assert_eq!(token_for('é'), None);
        assert_eq!(token_for('日'), None);
    }
}
