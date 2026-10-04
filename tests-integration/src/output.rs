use std::io::Read;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DIAGNOSTIC_TAIL_CHARS: usize = 2000;

#[derive(Clone)]
pub struct SharedOutput {
    inner: Arc<(Mutex<String>, Condvar)>,
}

impl SharedOutput {
    pub fn new() -> SharedOutput {
        SharedOutput {
            inner: Arc::new((Mutex::new(String::new()), Condvar::new())),
        }
    }

    pub fn push(&self, s: &str) {
        let (lock, cvar) = &*self.inner;
        let mut buffer = lock.lock().unwrap_or_else(|err| err.into_inner());
        buffer.push_str(s);
        cvar.notify_all();
    }

    pub fn wait_for(
        &self,
        pred: impl Fn(&str) -> bool,
        timeout: Duration,
    ) -> Result<usize, String> {
        let deadline = Instant::now() + timeout;
        let (lock, cvar) = &*self.inner;
        let mut buffer = lock.lock().unwrap_or_else(|err| err.into_inner());
        loop {
            let stripped = strip_ansi(&buffer);
            if pred(&stripped) {
                return Ok(buffer.len());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(tail_of(&stripped, DIAGNOSTIC_TAIL_CHARS));
            }
            let (next_buffer, wait_result) = cvar
                .wait_timeout(buffer, deadline - now)
                .unwrap_or_else(|err| err.into_inner());
            buffer = next_buffer;
            let _ = wait_result;
        }
    }

    pub fn expect_output(&self, substr: &str, timeout: Duration) {
        if let Err(diagnostic_tail) = self.wait_for(|buffer| buffer.contains(substr), timeout) {
            panic!(
                "timed out after {timeout:?} waiting for output containing {substr:?}\ncaptured output tail:\n{diagnostic_tail}"
            );
        }
    }

    pub fn tail(&self, n: usize) -> String {
        let stripped = strip_ansi(&self.snapshot());
        tail_of(&stripped, n)
    }

    pub fn len(&self) -> usize {
        self.snapshot().len()
    }

    fn snapshot(&self) -> String {
        let (lock, _cvar) = &*self.inner;
        lock.lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(esc_pos) = rest.find('\u{1B}') {
        out.push_str(&rest[..esc_pos]);
        let after_esc = &rest[esc_pos + 1..];
        if after_esc.is_empty() {
            return out;
        }
        if let Some(body) = after_esc.strip_prefix('[') {
            match body.find(|c: char| ('@'..='~').contains(&c)) {
                Some(final_pos) => rest = &body[final_pos + 1..],
                None => return out,
            }
        } else {
            let mut chars = after_esc.chars();
            chars.next();
            rest = chars.as_str();
        }
    }
    out.push_str(rest);
    out
}

pub fn spawn_reader<R: Read + Send + 'static>(mut reader: R, sink: SharedOutput) {
    thread::spawn(move || {
        let mut chunk = [0u8; 1024];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => sink.push(&String::from_utf8_lossy(&chunk[..n])),
                Err(_) => break,
            }
        }
    });
}

fn tail_of(stripped: &str, n: usize) -> String {
    let char_count = stripped.chars().count();
    if char_count <= n {
        return stripped.to_string();
    }
    stripped.chars().skip(char_count - n).collect()
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn strips_colored_prompt() {
        assert_eq!(strip_ansi("\x1B[32mrose>\x1B[m "), "rose> ");
    }

    #[test]
    fn strips_banner_color_prefix() {
        assert_eq!(strip_ansi("\x1B[40m\x1B[31m       _"), "       _");
    }

    #[test]
    fn strips_clear_and_home() {
        assert_eq!(strip_ansi("\x1B[2J\x1B[H"), "");
    }

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(strip_ansi("hello world\n"), "hello world\n");
    }

    #[test]
    fn trailing_escape_is_dropped() {
        assert_eq!(strip_ansi("abc\x1B"), "abc");
    }
}
