use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::disk_image;
use crate::kernel_build;
use crate::monitor::Monitor;
use crate::output::{self, SharedOutput};
use crate::sendkey;

const STDERR_TAIL_BYTES: usize = 4096;
const INTER_KEY_DELAY: Duration = Duration::from_millis(50);
const DROP_QUIT_CAP: Duration = Duration::from_secs(5);
const DROP_REAP_CAP: Duration = Duration::from_secs(5);
const DROP_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct QemuSession {
    child: Child,
    output: SharedOutput,
    stderr_tail: Arc<Mutex<String>>,
    monitor: Option<Monitor>,
    temp_dir: tempfile::TempDir,
}

impl QemuSession {
    pub fn spawn() -> QemuSession {
        let temp_dir = tempfile::TempDir::new().expect("failed to create temporary directory");
        let kernel_elf = kernel_build::kernel_elf();
        let disk_image = disk_image::create_disk_image(&kernel_elf, temp_dir.path());
        let monitor_socket = temp_dir.path().join("monitor.sock");

        let mut child = Command::new("qemu-system-x86_64")
            .arg("-drive")
            .arg(format!("format=raw,file={}", disk_image.display()))
            .arg("-debugcon")
            .arg("stdio")
            .arg("-display")
            .arg("none")
            .arg("-monitor")
            .arg(format!(
                "unix:{},server=on,wait=off",
                monitor_socket.display()
            ))
            .arg("-no-reboot")
            .arg("-serial")
            .arg("none")
            .arg("-parallel")
            .arg("none")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect(
                "failed to spawn qemu-system-x86_64; install QEMU and ensure it is on the PATH",
            );

        let output = SharedOutput::new();
        if let Some(stdout) = child.stdout.take() {
            output::spawn_reader(stdout, output.clone());
        }

        let stderr_tail = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            let sink = stderr_tail.clone();
            thread::spawn(move || {
                let mut reader = stderr;
                let mut chunk = [0u8; 1024];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut tail = sink.lock().unwrap_or_else(|err| err.into_inner());
                            tail.push_str(&String::from_utf8_lossy(&chunk[..n]));
                            if tail.len() > STDERR_TAIL_BYTES {
                                let mut cut = tail.len() - STDERR_TAIL_BYTES;
                                while !tail.is_char_boundary(cut) {
                                    cut += 1;
                                }
                                tail.replace_range(..cut, "");
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        QemuSession {
            child,
            output,
            stderr_tail,
            monitor: None,
            temp_dir,
        }
    }

    pub fn expect_output(&self, substr: &str, timeout: Duration) {
        if let Err(output_tail) = self.output.wait_for(|buffer| buffer.contains(substr), timeout) {
            let stderr_tail = self
                .stderr_tail
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone();
            panic!(
                "timed out after {timeout:?} waiting for output containing {substr:?}\ncaptured output tail:\n{output_tail}\nqemu stderr tail:\n{stderr_tail}"
            );
        }
    }

    pub fn send_key(&mut self, c: char) {
        let token = sendkey::token_for(c)
            .unwrap_or_else(|| panic!("cannot send character {c:?} over monitor sendkey"));
        self.monitor_mut()
            .send_command(&format!("sendkey {token}"))
            .unwrap_or_else(|err| panic!("monitor command sendkey {token} failed: {err}"));
    }

    pub fn send_text(&mut self, s: &str) {
        for c in s.chars() {
            self.send_key(c);
            thread::sleep(INTER_KEY_DELAY);
        }
    }

    pub fn send_line(&mut self, s: &str) {
        self.send_text(s);
        self.send_key('\n');
    }

    fn monitor_mut(&mut self) -> &mut Monitor {
        if self.monitor.is_none() {
            let socket = self.monitor_socket();
            self.monitor = Some(Monitor::connect(&socket).unwrap_or_else(|err| {
                panic!(
                    "failed to connect to QEMU monitor at {}: {err}",
                    socket.display()
                )
            }));
        }
        self.monitor.as_mut().unwrap()
    }

    fn monitor_socket(&self) -> PathBuf {
        self.temp_dir.path().join("monitor.sock")
    }
}

impl Drop for QemuSession {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor.as_mut() {
            let _ = monitor.quit();
        } else if let Ok(mut monitor) =
            Monitor::connect_with_cap(&self.monitor_socket(), DROP_QUIT_CAP)
        {
            let _ = monitor.quit();
        }

        let deadline = Instant::now() + DROP_REAP_CAP;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        return;
                    }
                    thread::sleep(DROP_POLL_INTERVAL);
                }
                Err(_) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
            }
        }
    }
}
