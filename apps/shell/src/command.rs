use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub struct Command {
    pub name: String,
    pub args: Vec<String>,
    pub background: bool,
}

impl Command {
    pub fn parse(line: &str) -> Option<Command> {
        let mut parts = line.split_whitespace();
        let name = parts.next()?.to_string();
        let mut args: Vec<String> = parts.map(|s| s.to_string()).collect();
        let background = matches!(args.last(), Some(token) if token == "&");
        if background {
            args.pop();
        }
        Some(Command { name, args, background })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse_empty_string_returns_none() {
        assert!(Command::parse("").is_none());
    }

    #[test]
    fn parse_single_word_returns_command_with_no_args() {
        let cmd = Command::parse("ls").unwrap();
        assert_eq!(cmd.name, "ls");
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn parse_command_with_args() {
        let cmd = Command::parse("echo hello world").unwrap();
        assert_eq!(cmd.name, "echo");
        assert_eq!(cmd.args, vec!["hello", "world"]);
    }

    #[test]
    fn parse_multiple_spaces_between_args() {
        let cmd = Command::parse("echo  hello   world").unwrap();
        assert_eq!(cmd.name, "echo");
        assert_eq!(cmd.args, vec!["hello", "world"]);
    }

    #[test]
    fn parse_leading_and_trailing_whitespace() {
        let cmd = Command::parse("  ls  ").unwrap();
        assert_eq!(cmd.name, "ls");
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn parse_trailing_ampersand_marks_background() {
        let cmd = Command::parse("pi &").unwrap();
        assert_eq!(cmd.name, "pi");
        assert!(cmd.args.is_empty());
        assert!(cmd.background);
    }

    #[test]
    fn parse_trailing_ampersand_after_arguments_keeps_them() {
        let cmd = Command::parse("snake fast 42 &").unwrap();
        assert_eq!(cmd.name, "snake");
        assert_eq!(cmd.args, vec!["fast", "42"]);
        assert!(cmd.background);
    }

    #[test]
    fn parse_ampersand_with_surrounding_whitespace_marks_background() {
        let cmd = Command::parse("pi  &  ").unwrap();
        assert!(cmd.background);
    }

    #[test]
    fn parse_ampersand_before_end_is_an_ordinary_argument() {
        let cmd = Command::parse("pi & 10").unwrap();
        assert_eq!(cmd.args, vec!["&", "10"]);
        assert!(!cmd.background);
    }

    #[test]
    fn parse_bare_ampersand_is_a_name_not_a_background_flag() {
        let cmd = Command::parse("&").unwrap();
        assert_eq!(cmd.name, "&");
        assert!(!cmd.background);
    }

    #[test]
    fn parse_ampersand_glued_to_name_is_part_of_the_name() {
        let cmd = Command::parse("pi&").unwrap();
        assert_eq!(cmd.name, "pi&");
        assert!(!cmd.background);
    }
}