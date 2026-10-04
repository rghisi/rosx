use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_CONNECT_CAP: Duration = Duration::from_secs(15);
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const PROMPT: &str = "(qemu) ";

pub struct Monitor {
    stream: UnixStream,
}

impl Monitor {
    pub fn connect(socket_path: &Path) -> std::io::Result<Monitor> {
        Self::connect_with_cap(socket_path, DEFAULT_CONNECT_CAP)
    }

    pub(crate) fn connect_with_cap(socket_path: &Path, cap: Duration) -> std::io::Result<Monitor> {
        let deadline = Instant::now() + cap;
        loop {
            match UnixStream::connect(socket_path) {
                Ok(stream) => return Ok(Monitor { stream }),
                Err(err) => {
                    if Instant::now() >= deadline {
                        return Err(err);
                    }
                    thread::sleep(CONNECT_RETRY_INTERVAL);
                }
            }
        }
    }

    pub fn send_command(&mut self, cmd: &str) -> std::io::Result<String> {
        self.stream.set_read_timeout(Some(READ_TIMEOUT))?;
        self.stream.write_all(cmd.as_bytes())?;
        self.stream.write_all(b"\n")?;
        self.stream.flush()?;
        self.read_until_prompt()
    }

    pub fn quit(&mut self) -> std::io::Result<String> {
        match self.send_command("quit") {
            Ok(reply) => Ok(reply),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::NotConnected
                ) =>
            {
                Ok(String::new())
            }
            Err(err) => Err(err),
        }
    }

    fn read_until_prompt(&mut self) -> std::io::Result<String> {
        let mut reply = String::new();
        let mut chunk = [0u8; 1024];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(reply),
                Ok(n) => {
                    reply.push_str(&String::from_utf8_lossy(&chunk[..n]));
                    if reply.contains(PROMPT) {
                        return Ok(reply);
                    }
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(reply);
                }
                Err(err) => return Err(err),
            }
        }
    }
}
