use std::collections::BTreeSet;
use std::env;
use std::time::{Duration, Instant};

use crate::qemu::QemuSession;

const PROMPT: &str = "rose>";
const PS_TABLE_MARKER: &str = "processes,";
const BOOT_TIMEOUT: Duration = Duration::from_secs(90);
const PS_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSCRIPT_CAP: usize = 4000;

const IPD_READY: &str = "[IPD] bound IP";
const IPD_ATTEMPTS: usize = 5;
const IPD_BUDGET: Duration = Duration::from_secs(60);

pub fn start_ipd(session: &mut QemuSession) {
    session.run_command("/srv/ipd &", |delta| delta.contains(IPD_READY), IPD_ATTEMPTS, IPD_BUDGET);
}

pub fn process_names(report: &str) -> BTreeSet<String> {    report
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 5 {
                return None;
            }
            fields[0].parse::<u32>().ok()?;
            fields[3].parse::<u64>().ok()?;
            let name = fields[4..].join(" ");
            (name != "ps").then_some(name)
        })
        .collect()
}

pub fn process_set_drift(baseline: &BTreeSet<String>, current: &BTreeSet<String>) -> Option<String> {
    let strays: Vec<&String> = current.difference(baseline).collect();
    let missing: Vec<&String> = baseline.difference(current).collect();
    if strays.is_empty() && missing.is_empty() {
        return None;
    }
    let mut message = String::from("process set drifted:");
    if !strays.is_empty() {
        message.push_str(&format!(" still alive: [{}]", join(strays)));
    }
    if !missing.is_empty() {
        message.push_str(&format!(" gone missing: [{}]", join(missing)));
    }
    Some(message)
}

fn join(items: Vec<&String>) -> String {
    items
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<&str>>()
        .join(", ")
}

pub struct SharedSession {
    group: &'static str,
    session: QemuSession,
    last_case: String,
    baseline: BTreeSet<String>,
}

impl SharedSession {
    pub fn spawn(group: &'static str) -> SharedSession {
        let mut session = QemuSession::spawn();
        session.expect_output(PROMPT, BOOT_TIMEOUT);
        session.expect_disk_boot();
        let baseline = take_process_set(&mut session);
        SharedSession {
            group,
            session,
            last_case: String::from("boot"),
            baseline,
        }
    }

    pub fn session(&mut self) -> &mut QemuSession {
        &mut self.session
    }

    pub fn case(&mut self, name: &str, body: impl FnOnce(&mut QemuSession)) {
        let timing = env::var_os("ROSX_SHARED_TIMING").is_some_and(|value| !value.is_empty());
        if self.last_case != "boot" {
            let reset_started = Instant::now();
            self.reset();
            if timing {
                eprintln!(
                    "[shared-timing] {}/{} reset {:?}",
                    self.group,
                    name,
                    reset_started.elapsed()
                );
            }
        }
        self.last_case = name.to_string();
        let body_started = Instant::now();
        body(&mut self.session);
        if timing {
            eprintln!(
                "[shared-timing] {}/{} body {:?}",
                self.group,
                name,
                body_started.elapsed()
            );
        }
    }

    pub fn reset(&mut self) {
        let mark = self.session.mark();
        self.session.send_key('\x03');
        self.session.send_line("ps");
        let after_header = self
            .session
            .wait_until_since(mark, |delta| delta.contains(PS_TABLE_MARKER), PS_TIMEOUT)
            .unwrap_or_else(|_| self.reset_failed("`ps` printed no table", mark));
        if self
            .session
            .wait_until_since(after_header, |delta| delta.contains(PROMPT), PS_TIMEOUT)
            .is_err()
        {
            self.reset_failed("the `ps` table never ended at a prompt", mark);
        }
        let current = process_names(&self.session.content_since(mark));
        if let Some(drift) = process_set_drift(&self.baseline, &current) {
            self.reset_failed(&drift, mark);
        }
    }

    fn reset_failed(&self, why: &str, mark: usize) -> ! {
        panic!(
            "RESET FAILURE in group {:?}, after case {:?}: {why}\nto fix: this VM is dirty or wedged, the next test must not run on it\ntranscript since the reset mark:\n{}\nqemu stderr tail:\n{}",
            self.group,
            self.last_case,
            cap(&self.session.content_since(mark), TRANSCRIPT_CAP),
            self.session.stderr_tail(),
        );
    }
}

fn take_process_set(session: &mut QemuSession) -> BTreeSet<String> {
    let mark = session.mark();
    let report = session.run_command("ps", |delta| delta.contains(PS_TABLE_MARKER), 1, PS_TIMEOUT);
    session.expect_output_since(mark, PROMPT, PS_TIMEOUT);
    process_names(&report)
}

fn cap(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    text.chars().skip(count - limit).collect()
}

#[cfg(test)]
mod tests {
    use super::{cap, process_names, process_set_drift};

    const PS_TABLE: &str = concat!(
        " PID  STATE   THREADS   MEM(KiB)  NAME\n",
        "   4  ready         1       104  [K] Kstatsd\n",
        "   5  ready         1        52  atad\n",
        "   9  blocked       1        56  vfsd\n",
        "  17  ready         1        60  shell\n",
        "  18  ready         1        52  ps\n",
        "6 processes, 376 KiB accounted\n",
    );

    fn set(names: &[&str]) -> std::collections::BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn parses_rows_skipping_header_and_footer() {
        let names = process_names(PS_TABLE);
        assert!(names.contains("[K] Kstatsd"), "whitespace names survive: {names:?}");
        assert!(names.contains("shell"));
        assert!(!names.contains("ps"), "the probe itself is not a resident process");
        assert!(!names.contains("NAME"));
        assert!(!names.contains("6 processes, 376 KiB accounted"));
        assert_eq!(names.len(), 4);
    }

    #[test]
    fn tolerates_fragmented_and_interleaved_text() {
        let names = process_names("rose> ps\n  12  ready  1  52  exiter\nnoise\n  13  ready  1  52  spinner\n4 processes, 100 KiB accounted\nrose> ");
        assert!(names.contains("exiter"));
        assert!(names.contains("spinner"));
        assert!(!names.contains("noise"));
    }

    #[test]
    fn equal_sets_drift_nothing() {
        let baseline = set(&["shell", "vfsd"]);
        assert!(process_set_drift(&baseline, &set(&["vfsd", "shell"])).is_none());
    }

    #[test]
    fn reports_strays_and_missing() {
        let baseline = set(&["shell", "vfsd"]);
        let drift = process_set_drift(&baseline, &set(&["shell", "spinner"])).unwrap();
        assert!(drift.contains("still alive: [spinner]"), "{drift}");
        assert!(drift.contains("gone missing: [vfsd]"), "{drift}");
    }

    #[test]
    fn strays_alone_are_a_drift() {
        let drift = process_set_drift(&set(&["shell"]), &set(&["shell", "pi"])).unwrap();
        assert!(drift.contains("still alive: [pi]"));
        assert!(!drift.contains("gone missing"));
    }

    #[test]
    fn cap_keeps_the_tail() {
        assert_eq!(cap("abcdef", 4), "cdef");
        assert_eq!(cap("abc", 10), "abc");
    }
}
