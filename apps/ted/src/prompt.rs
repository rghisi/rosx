use alloc::string::String;
use core::mem;

use input_protocol::wire::{is_arrow_sentinel, KEY_ESCAPE};
use vfs_protocol::wire::MAX_NAME_BYTES;

pub const MAX_TYPED_BYTES: usize = MAX_NAME_BYTES - 1;

pub enum PromptStep {
    Continue,
    Committed(String),
    Cancelled,
}

pub struct NamePrompt {
    typed: String,
}

impl NamePrompt {
    pub fn new() -> NamePrompt {
        NamePrompt { typed: String::new() }
    }

    pub fn text(&self) -> &str {
        &self.typed
    }

    pub fn input(&mut self, c: char) -> PromptStep {
        if c == KEY_ESCAPE {
            return PromptStep::Cancelled;
        }
        match c {
            '\n' | '\r' if self.typed.is_empty() => PromptStep::Cancelled,
            '\n' | '\r' => PromptStep::Committed(mem::take(&mut self.typed)),
            '\x08' | '\u{7F}' => {
                self.typed.pop();
                PromptStep::Continue
            }
            c if c >= '\u{20}' && c != '\u{7F}' && !is_arrow_sentinel(c)
                && self.typed.len() + c.len_utf8() <= MAX_TYPED_BYTES =>
            {
                self.typed.push(c);
                PromptStep::Continue
            }
            _ => PromptStep::Continue,
        }
    }
}

impl Default for NamePrompt {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_protocol::wire::KEY_ARROW_UP;

    fn commit_of(step: PromptStep) -> String {
        match step {
            PromptStep::Committed(name) => name,
            _ => panic!("expected a committed name"),
        }
    }

    fn is_cancelled(step: PromptStep) -> bool {
        matches!(step, PromptStep::Cancelled)
    }

    fn is_continue(step: PromptStep) -> bool {
        matches!(step, PromptStep::Continue)
    }

    #[test]
    fn printable_characters_accumulate() {
        let mut prompt = NamePrompt::new();
        assert!(is_continue(prompt.input('n')));
        assert!(is_continue(prompt.input('o')));
        assert!(is_continue(prompt.input('t')));
        assert!(is_continue(prompt.input('e')));
        assert!(is_continue(prompt.input('s')));
        assert!(is_continue(prompt.input('.')));
        assert!(is_continue(prompt.input('t')));
        assert!(is_continue(prompt.input('x')));
        assert!(is_continue(prompt.input('t')));
        assert_eq!(prompt.text(), "notes.txt");
        assert_eq!(commit_of(prompt.input('\n')), "notes.txt");
        assert_eq!(prompt.text(), "");
    }

    #[test]
    fn carriage_return_commits_like_enter() {
        let mut prompt = NamePrompt::new();
        prompt.input('/');
        prompt.input('a');
        assert_eq!(commit_of(prompt.input('\r')), "/a");
    }

    #[test]
    fn backspace_edits_the_typed_name() {
        let mut prompt = NamePrompt::new();
        prompt.input('a');
        prompt.input('b');
        assert!(is_continue(prompt.input('\x08')));
        assert_eq!(prompt.text(), "a");
        assert!(is_continue(prompt.input('\u{7F}')));
        assert_eq!(prompt.text(), "");
        assert!(is_continue(prompt.input('\x08')));
    }

    #[test]
    fn escape_cancels_and_empty_enter_cancels() {
        let mut prompt = NamePrompt::new();
        prompt.input('a');
        assert!(is_cancelled(prompt.input(KEY_ESCAPE)));
        assert_eq!(prompt.text(), "a");
        let mut fresh = NamePrompt::new();
        assert!(is_cancelled(fresh.input('\n')));
    }

    #[test]
    fn control_keys_and_arrow_sentinels_are_ignored() {
        let mut prompt = NamePrompt::new();
        assert!(is_continue(prompt.input(KEY_ARROW_UP)));
        assert!(is_continue(prompt.input('\x13')));
        assert!(is_continue(prompt.input('\t')));
        assert_eq!(prompt.text(), "");
    }

    #[test]
    fn typing_stops_at_the_wire_name_budget() {
        let mut prompt = NamePrompt::new();
        for _ in 0..MAX_TYPED_BYTES {
            assert!(is_continue(prompt.input('x')));
        }
        assert!(is_continue(prompt.input('x')));
        assert_eq!(prompt.text().len(), MAX_TYPED_BYTES);
    }
}
