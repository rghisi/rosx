use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub const RECORD_PREFIX: &str = "bench-x86_64-";
pub const HISTORY_DIRECTORY: &str = "benchmarks";

const MEASUREMENT_FIELDS: [&str; 5] = [
    "iterations=",
    "elapsed_ms=",
    "ops_per_sec=",
    "ns_per_op=",
    "reps=",
];

pub type Measurements = BTreeMap<String, u64>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub recorded_at_utc: String,
    pub host: String,
    pub host_cpus: usize,
    pub cpu_model: String,
    pub mem_total_kb: u64,
    pub virtualization: String,
    pub kvm_available: bool,
    pub kernel_profile: String,
    pub qemu_version: String,
    pub git_commit: String,
}

impl Context {
    pub fn detect_now() -> Context {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        Context {
            recorded_at_utc: format_utc(seconds),
            host: detect_host(),
            host_cpus: detect_host_cpus(),
            cpu_model: detect_cpu_model(),
            mem_total_kb: detect_mem_total_kb(),
            virtualization: detect_virtualization(),
            kvm_available: detect_kvm_available(),
            kernel_profile: detect_kernel_profile(),
            qemu_version: detect_qemu_version(),
            git_commit: detect_git_commit(),
        }
    }

    pub fn header(&self) -> String {
        format!(
            "# rosx-bench-record v3\n\
             # recorded_at_utc={}\n\
             # host={}\n\
             # host_cpus={}\n\
             # cpu_model={}\n\
             # mem_total_kb={}\n\
             # virtualization={}\n\
             # kvm_available={}\n\
             # kernel_profile={}\n\
             # qemu={}\n\
             # git_commit={}\n",
            self.recorded_at_utc,
            self.host,
            self.host_cpus,
            self.cpu_model,
            self.mem_total_kb,
            self.virtualization,
            yes_no(self.kvm_available),
            self.kernel_profile,
            self.qemu_version,
            self.git_commit
        )
    }

    fn comparable(&self) -> Vec<(&'static str, String)> {
        vec![
            ("host", self.host.clone()),
            ("host_cpus", self.host_cpus.to_string()),
            ("cpu_model", self.cpu_model.clone()),
            ("mem_total_kb", self.mem_total_kb.to_string()),
            ("virtualization", self.virtualization.clone()),
            ("kvm_available", yes_no(self.kvm_available).to_string()),
            ("kernel_profile", self.kernel_profile.clone()),
            ("qemu", self.qemu_version.clone()),
        ]
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

pub fn parse_bench_lines(output: &str) -> Measurements {
    let tokens: Vec<&str> = output.split_whitespace().collect();
    let mut measurements = Measurements::new();
    let mut index = 0;
    while index < tokens.len() {
        let Some(scenario) = tokens[index]
            .strip_prefix("bench.")
            .and_then(|token| token.strip_suffix(".best"))
        else {
            index += 1;
            continue;
        };
        let mut identity = vec![scenario];
        let mut value: Option<u64> = None;
        index += 1;
        while index < tokens.len() && !tokens[index].starts_with("bench.") {
            let token = tokens[index];
            if let Some(parsed) = token.strip_prefix("ops_per_sec=") {
                value = parsed.parse().ok();
            }
            if !MEASUREMENT_FIELDS
                .iter()
                .any(|field| token.starts_with(field))
            {
                identity.push(token);
            }
            index += 1;
            if token.starts_with("reps=") {
                break;
            }
        }
        if let Some(parsed) = value {
            measurements.insert(identity.join(" "), parsed);
        }
    }
    measurements
}

pub fn render_record(context: &Context, measurements: &Measurements) -> String {
    let mut entries: Vec<(&String, &u64)> = measurements.iter().collect();
    entries.sort_by(|(left, _), (right, _)| compare_identities(left, right));
    let mut record = context.header();
    for (identity, value) in entries {
        record.push_str(&format!("{identity} ops_per_sec={value}\n"));
    }
    record
}

fn compare_identities(left: &str, right: &str) -> std::cmp::Ordering {
    let mut left_tokens = left.split_whitespace();
    let mut right_tokens = right.split_whitespace();
    loop {
        match (left_tokens.next(), right_tokens.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(l), Some(r)) => {
                let number_order = numeric_value(l).zip(numeric_value(r)).map(|(l, r)| l.cmp(&r));
                match number_order.unwrap_or_else(|| l.cmp(r)) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                }
            }
        }
    }
}

fn numeric_value(token: &str) -> Option<u64> {
    token.rsplit('=').next()?.parse().ok()
}

pub fn parse_record(record: &str) -> (BTreeMap<String, String>, Measurements) {
    let mut header = BTreeMap::new();
    let mut measurements = Measurements::new();
    for line in record.lines() {
        let trimmed = line.trim();
        if let Some(entry) = trimmed.strip_prefix("# ") {
            if let Some((key, value)) = entry.split_once('=') {
                header.insert(key.to_string(), value.to_string());
            }
        } else if let Some(index) = trimmed.rfind(" ops_per_sec=") {
            let identity = &trimmed[..index];
            let value = trimmed[index + " ops_per_sec=".len()..].trim();
            if let Ok(parsed) = value.parse::<u64>() {
                measurements.insert(identity.to_string(), parsed);
            }
        }
    }
    (header, measurements)
}

pub fn context_mismatches(current: &Context, recorded: &BTreeMap<String, String>) -> Vec<String> {
    current
        .comparable()
        .into_iter()
        .filter(|(key, value)| recorded.get(*key).is_none_or(|stored| stored != value))
        .map(|(key, value)| format!("{key} differs: recorded {:?}, now {value:?}", recorded.get(key)))
        .collect()
}

pub fn compare(
    baseline: &Measurements,
    current: &Measurements,
    tolerance_pct: u64,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (identity, baseline_value) in baseline {
        match current.get(identity) {
            None => problems.push(format!("missing from current run: {identity}")),
            Some(value) => {
                let floor = baseline_value.saturating_mul(100 - tolerance_pct.min(100));
                if value.saturating_mul(100) < floor {
                    let drop = baseline_value
                        .saturating_sub(*value)
                        .saturating_mul(100)
                        / baseline_value.max(&1);
                    problems.push(format!(
                        "{identity} ops_per_sec {value} vs baseline {baseline_value} (-{drop}%)"
                    ));
                }
            }
        }
    }
    for identity in current.keys() {
        if !baseline.contains_key(identity) {
            problems.push(format!("new measurement not in baseline: {identity}"));
        }
    }
    problems
}

pub fn utc_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

pub fn record_filename(seconds: u64, pull_request: Option<&str>) -> String {
    let (year, month, day, hour, minute, second) = civil_from_epoch_seconds(seconds);
    match pull_request {
        Some(number) => format!(
            "{RECORD_PREFIX}{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}-pr{number}.txt"
        ),
        None => format!(
            "{RECORD_PREFIX}{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}.txt"
        ),
    }
}

pub fn history_directory() -> PathBuf {
    repo_root().join(HISTORY_DIRECTORY)
}

pub fn append_record(
    directory: &Path,
    context: &Context,
    measurements: &Measurements,
    pull_request: Option<&str>,
) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let path = directory.join(record_filename(utc_timestamp_now(), pull_request));
    fs::write(&path, render_record(context, measurements))?;
    Ok(path)
}

pub fn latest_record(directory: &Path) -> Option<PathBuf> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(RECORD_PREFIX) && name.ends_with(".txt"))
        .collect();
    names.sort();
    names.last().map(|name| directory.join(name))
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn civil_from_epoch_seconds(seconds: u64) -> (i64, u32, u32, u32, u32, u32) {
    let (year, month, day) = civil_from_days((seconds / 86400) as i64);
    let second_of_day = (seconds % 86400) as u32;
    (
        year,
        month,
        day,
        second_of_day / 3600,
        second_of_day % 3600 / 60,
        second_of_day % 60,
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let shifted = days_since_epoch + 719_468;
    let era = if shifted >= 0 { shifted } else { shifted - 146_096 } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn format_utc(seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = civil_from_epoch_seconds(seconds);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn detect_host() -> String {
    if let Ok(name) = env::var("HOSTNAME") {
        if !name.is_empty() {
            return name;
        }
    }
    fs::read_to_string("/etc/hostname")
        .map(|name| name.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn detect_host_cpus() -> usize {
    thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(0)
}

fn detect_cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|cpuinfo| {
            cpuinfo
                .lines()
                .find_map(|line| line.strip_prefix("model name"))
                .and_then(|rest| rest.split_once(':'))
                .map(|(_, model)| model.trim().to_string())
        })
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn detect_mem_total_kb() -> u64 {
    fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|meminfo| {
            meminfo
                .lines()
                .find(|line| line.starts_with("MemTotal:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|kb| kb.parse().ok())
        })
        .unwrap_or(0)
}

fn detect_virtualization() -> String {
    if let Ok(output) = Command::new("systemd-detect-virt").output() {
        if output.status.success() {
            let kind = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !kind.is_empty() {
                return kind;
            }
        }
    }
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|cpuinfo| {
            cpuinfo
                .lines()
                .find(|line| line.starts_with("flags"))
                .and_then(|line| {
                    line.split_whitespace()
                        .any(|flag| flag == "hypervisor")
                        .then(|| "hypervisor".to_string())
                })
        })
        .unwrap_or_else(|| "none".to_string())
}

fn detect_kvm_available() -> bool {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

fn detect_kernel_profile() -> String {
    if env::var_os("ROSX_KERNEL_ELF").is_some_and(|value| !value.is_empty()) {
        return "custom".to_string();
    }
    env::var("ROSX_QEMU_PROFILE").unwrap_or_else(|_| "dev".to_string())
}

fn detect_qemu_version() -> String {
    Command::new("qemu-system-x86_64")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.lines().next().map(|line| line.trim().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

fn detect_git_commit() -> String {
    Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .arg("rev-parse")
        .arg("--short")
        .arg("HEAD")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|commit| commit.trim().to_string())
        .filter(|commit| !commit.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_output() -> &'static str {
        "rose> bench\n\
         Running: alloc, yield\n\
         bench.start suite=alloc shape=pairs align=8\n\
         bench.alloc size=64 align=8 rep=1 iterations=25600 elapsed_ms=500 ops_per_sec=51200 ns_per_op=19531\n\
         bench.alloc size=64 align=8 rep=2 iterations=25600 elapsed_ms=490 ops_per_sec=52244 ns_per_op=19140\n\
         bench.alloc.best size=64 align=8 iterations=25600 elapsed_ms=490 ops_per_sec=52244 ns_per_op=19140 reps=3\n\
         bench.alloc size=4194304 align=8 rep=1 iterations=25600 elapsed_ms=540 ops_per_sec=47407 ns_per_op=21093\n\
         bench.alloc.best size=4194304 align=8 iterations=25600 elapsed_ms=530 ops_per_sec=48301 ns_per_op=20703 reps=3\n\
         bench.done suite=alloc\n\
         bench.start suite=yield shape=voluntary\n\
         bench.yield.best shape=voluntary iterations=1600 elapsed_ms=300 ops_per_sec=5333 ns_per_op=187500 reps=3\n\
         bench.done suite=yield\n\
         === Bench Finished ===\n"
    }

    fn sample_context() -> Context {
        Context {
            recorded_at_utc: "2026-11-01T12:00:00Z".to_string(),
            host: "testhost".to_string(),
            host_cpus: 8,
            cpu_model: "Test CPU @ 3.00GHz".to_string(),
            mem_total_kb: 33_554_432,
            virtualization: "kvm".to_string(),
            kvm_available: false,
            kernel_profile: "dev".to_string(),
            qemu_version: "QEMU emulator version 9.2.0".to_string(),
            git_commit: "abc1234".to_string(),
        }
    }

    #[test]
    fn parses_only_best_lines_into_identity_and_rate() {
        let measurements = parse_bench_lines(sample_output());
        assert_eq!(measurements.len(), 3);
        assert_eq!(measurements["alloc size=64 align=8"], 52244);
        assert_eq!(measurements["alloc size=4194304 align=8"], 48301);
        assert_eq!(measurements["yield shape=voluntary"], 5333);
    }

    #[test]
    fn ignores_output_without_best_lines() {
        assert!(parse_bench_lines("rose> bench list\nAvailable benchmarks: alloc, yield\n").is_empty());
    }

    #[test]
    fn record_roundtrips_through_render_and_parse() {
        let measurements = parse_bench_lines(sample_output());
        let context = sample_context();
        let (header, parsed) = parse_record(&render_record(&context, &measurements));
        assert_eq!(header["recorded_at_utc"], "2026-11-01T12:00:00Z");
        assert_eq!(header["host"], "testhost");
        assert_eq!(header["host_cpus"], "8");
        assert_eq!(header["cpu_model"], "Test CPU @ 3.00GHz");
        assert_eq!(header["mem_total_kb"], "33554432");
        assert_eq!(header["virtualization"], "kvm");
        assert_eq!(header["kvm_available"], "no");
        assert_eq!(header["kernel_profile"], "dev");
        assert_eq!(parsed, measurements);
    }

    #[test]
    fn flags_regression_beyond_tolerance_within_tolerance_is_clean() {
        let mut baseline = Measurements::new();
        baseline.insert("alloc size=64 align=8".to_string(), 100);
        baseline.insert("alloc size=128 align=8".to_string(), 100);
        baseline.insert("yield shape=voluntary".to_string(), 100);

        let mut current = Measurements::new();
        current.insert("alloc size=64 align=8".to_string(), 50);
        current.insert("alloc size=128 align=8".to_string(), 85);
        current.insert("yield shape=voluntary".to_string(), 80);

        let problems = compare(&baseline, &current, 20);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("alloc size=64"));
        assert!(problems[0].contains("-50%"));
    }

    #[test]
    fn reports_identity_present_on_only_one_side() {
        let mut baseline = Measurements::new();
        baseline.insert("alloc size=4096 align=8".to_string(), 100);
        let mut current = Measurements::new();
        current.insert("yield shape=voluntary".to_string(), 100);

        let problems = compare(&baseline, &current, 20);
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().any(|problem| problem.starts_with("missing from current run: alloc size=4096")));
        assert!(problems.iter().any(|problem| problem.starts_with("new measurement not in baseline: yield")));
    }

    #[test]
    fn identical_measurements_have_no_problems() {
        let measurements = parse_bench_lines(sample_output());
        assert!(compare(&measurements, &measurements, 20).is_empty());
    }

    #[test]
    fn context_mismatch_detected_for_comparable_fields_only() {
        let context = Context {
            kernel_profile: "release".to_string(),
            ..sample_context()
        };
        let mut header = BTreeMap::new();
        header.insert("host".to_string(), "testhost".to_string());
        header.insert("host_cpus".to_string(), "8".to_string());
        header.insert("cpu_model".to_string(), "Test CPU @ 3.00GHz".to_string());
        header.insert("mem_total_kb".to_string(), "33554432".to_string());
        header.insert("virtualization".to_string(), "kvm".to_string());
        header.insert("kvm_available".to_string(), "no".to_string());
        header.insert("kernel_profile".to_string(), "dev".to_string());
        header.insert("qemu".to_string(), "QEMU emulator version 9.2.0".to_string());
        let mismatches = context_mismatches(&context, &header);
        assert_eq!(mismatches.len(), 1);
        assert!(mismatches[0].contains("kernel_profile"));
    }

    #[test]
    fn missing_hardware_context_keys_are_reported_as_mismatches() {
        let context = sample_context();
        let mut header = BTreeMap::new();
        header.insert("host".to_string(), "testhost".to_string());
        header.insert("kernel_profile".to_string(), "dev".to_string());
        header.insert("qemu".to_string(), "QEMU emulator version 9.2.0".to_string());
        let mismatches = context_mismatches(&context, &header);
        assert_eq!(mismatches.len(), 5);
        assert!(mismatches.iter().all(|problem| problem.contains("differs")));
    }

    #[test]
    fn filenames_sort_chronologically_with_optional_pr_tag() {
        let plain = record_filename(1_762_000_000, None);
        let tagged = record_filename(1_762_000_000, Some("42"));
        let later = record_filename(1_762_000_061, None);
        assert!(plain.starts_with(RECORD_PREFIX));
        assert!(plain.ends_with(".txt"));
        assert!(tagged.contains("-pr42.txt"));
        assert!(tagged < plain);
        assert!(plain < later);
    }

    #[test]
    fn latest_record_picks_lexicographic_last() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("bench-x86_64-20260101-000000.txt"), "").unwrap();
        fs::write(directory.path().join("bench-x86_64-20261231-235959.txt"), "").unwrap();
        fs::write(directory.path().join("unrelated.txt"), "").unwrap();
        let latest = latest_record(directory.path()).unwrap();
        assert_eq!(
            latest.file_name().unwrap().to_str().unwrap(),
            "bench-x86_64-20261231-235959.txt"
        );
    }

    #[test]
    fn append_record_writes_unique_timestamped_files() {
        let directory = tempfile::tempdir().unwrap();
        let context = sample_context();
        let mut measurements = Measurements::new();
        measurements.insert("yield shape=voluntary".to_string(), 5333);
        let first = append_record(directory.path(), &context, &measurements, None).unwrap();
        assert!(first.is_file());
        let (header, parsed) = parse_record(&fs::read_to_string(&first).unwrap());
        assert_eq!(parsed, measurements);
        assert_eq!(header["host"], "testhost");
    }

    #[test]
    fn rendered_records_order_sizes_numerically() {
        let context = sample_context();
        let mut measurements = Measurements::new();
        measurements.insert("alloc size=1024 align=8".to_string(), 1);
        measurements.insert("alloc size=64 align=8".to_string(), 1);
        measurements.insert("alloc size=4194304 align=8".to_string(), 1);
        measurements.insert("yield shape=voluntary".to_string(), 1);
        let rendered = render_record(&context, &measurements);
        let positions: Vec<usize> = ["size=64 ", "size=1024 ", "size=4194304 ", "yield "]
            .iter()
            .map(|needle| rendered.find(needle).unwrap())
            .collect();
        assert_eq!(positions, {
            let mut sorted = positions.clone();
            sorted.sort();
            sorted
        });
    }

    #[test]
    fn hardware_detection_returns_plausible_values_on_linux_hosts() {
        assert!(detect_host_cpus() >= 1);
        assert_ne!(detect_cpu_model(), "unknown");
        assert!(detect_mem_total_kb() > 0);
        assert!(!detect_virtualization().is_empty());
        let _ = detect_kvm_available();
    }

    #[test]
    fn civil_time_conversion_matches_known_instants() {
        assert_eq!(civil_from_epoch_seconds(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(
            civil_from_epoch_seconds(1_000_000_000),
            (2001, 9, 9, 1, 46, 40)
        );
        assert_eq!(
            civil_from_epoch_seconds(1_893_456_000),
            (2030, 1, 1, 0, 0, 0)
        );
    }
}
