use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub struct Command {
    pub name: String,
    pub args: Vec<String>,
}

impl Command {
    pub fn parse(line: &str) -> Option<Command> {
        let mut parts = line.split_whitespace();
        let name = parts.next()?.to_string();
        let args = parts.map(|s| s.to_string()).collect();
        Some(Command { name, args })
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
}