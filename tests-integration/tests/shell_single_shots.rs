use std::time::Duration;
use tests_integration::qemu::QemuSession;
use tests_integration::shared::SharedSession;

const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);
const ARGUMENTS_TIMEOUT: Duration = Duration::from_secs(30);
const LEAK_TIMEOUT: Duration = Duration::from_secs(30);
const REPORT_TIMEOUT: Duration = Duration::from_secs(15);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const REPORT_LINES: [&str; 6] = [
    "KSTATSD memory snapshot",
    "used:",
    "free:",
    "total:",
    "largest free block:",
    "free blocks:",
];
const ROUND_PREFIX: &str = "waitset round: ";
const NEGATIVE_PREFIX: &str = "waitset negative: ";
const LAST_LINE: &str = "waitset complete: 3 rounds ok";
const FORBIDDEN_STRINGS: [&str; 2] = ["fail", "Panic"];
const EXPECTED_NEGATIVE_LINES: [&str; 5] = [
    "waitset negative: nesting rejected ok",
    "waitset negative: child rejected ok",
    "waitset negative: waitfuture guarded ok",
    "waitset negative: detach purged ok",
    "waitset negative: drop idempotent ok",
];
const EXPECTED_TAGS: [&str; 3] = ["11", "22", "33"];
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;

#[test]
fn shell_single_shots_share_one_session() {
    let mut group = SharedSession::spawn("shell_single_shots");
    group.case("mem_reports_a_memory_snapshot_over_ipc", mem_reports_a_memory_snapshot_over_ipc);
    group.case("elf_app_receives_arguments_from_kernel", elf_app_receives_arguments_from_kernel);
    group.case(
        "shell_forwards_command_arguments_to_elf_app",
        shell_forwards_command_arguments_to_elf_app,
    );
    group.case(
        "a_dying_group_forfeits_its_abandoned_futures_and_the_pool_stays_usable",
        a_dying_group_forfeits_its_abandoned_futures_and_the_pool_stays_usable,
    );
    group.case("shell_waitset_or_composition_roundtrip", shell_waitset_or_composition_roundtrip);
}

fn mem_reports_a_memory_snapshot_over_ipc(session: &mut QemuSession) {
    let mark = session.mark();
    session.send_line("mem");
    for needle in REPORT_LINES {
        session.expect_output_since(mark, needle, SNAPSHOT_TIMEOUT);
    }

    let after_report = session.mark();
    session.expect_output_since(after_report, "rose>", SNAPSHOT_TIMEOUT);
}

fn elf_app_receives_arguments_from_kernel(session: &mut QemuSession) {
    let mark = session.mark();
    session.send_line("pi");
    session.expect_output_since(mark, "args(1): [\"pi\"]", ARGUMENTS_TIMEOUT);
}

fn shell_forwards_command_arguments_to_elf_app(session: &mut QemuSession) {
    let mark = session.mark();
    session.send_line("pi alpha beta");
    session.expect_output_since(
        mark,
        "args(3): [\"pi\", \"alpha\", \"beta\"]",
        ARGUMENTS_TIMEOUT,
    );
}

fn abandoned_count(line: &str) -> usize {
    line.split_whitespace()
        .nth(2)
        .and_then(|count| count.parse().ok())
        .unwrap_or_else(|| panic!("no abandoned count in {line:?}"))
}

fn a_dying_group_forfeits_its_abandoned_futures_and_the_pool_stays_usable(session: &mut QemuSession) {
    let first_run = session.mark();
    session.send_line("leaky");
    session.expect_output_since(first_run, "LEAKY ABANDONED ", LEAK_TIMEOUT);
    session.expect_output_since(first_run, "rose>", LEAK_TIMEOUT);
    let first = abandoned_count(
        session
            .content_since(first_run)
            .lines()
            .find(|line| line.contains("LEAKY ABANDONED "))
            .unwrap(),
    );
    assert!(first > 512, "leaky must have filled most of the pool, got {first}");

    let second_run = session.mark();
    session.send_line("leaky");
    session.expect_output_since(second_run, "LEAKY ABANDONED ", LEAK_TIMEOUT);
    session.expect_output_since(second_run, "rose>", LEAK_TIMEOUT);
    let second = abandoned_count(
        session
            .content_since(second_run)
            .lines()
            .find(|line| line.contains("LEAKY ABANDONED "))
            .unwrap(),
    );
    assert!(
        second + 16 >= first,
        "the dead group must have returned its slots: first abandoned {first}, second only {second}"
    );

    let after_report = session.mark();
    session.send_line("exiter 7");
    session.expect_output_since(after_report, "exiter exited with code 7", REPORT_TIMEOUT);
    session.expect_output_since(after_report, "rose>", REPORT_TIMEOUT);
}

fn count_prefixes(content: &str, prefix: &str) -> usize {
    content
        .lines()
        .filter(|line| line.starts_with(prefix))
        .count()
}

fn tail_chars(content: &str, n: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= n {
        return content.to_string();
    }
    content.chars().skip(char_count - n).collect()
}

fn window_diagnostics(content: &str) -> String {
    format!(
        "round_line_count={} negative_line_count={} last_line_present={} window_tail={}",
        count_prefixes(content, ROUND_PREFIX),
        count_prefixes(content, NEGATIVE_PREFIX),
        content.contains(LAST_LINE),
        tail_chars(content, DIAGNOSTIC_TAIL_CHARS)
    )
}

fn round_tokens(content: &str) -> Vec<Vec<&str>> {
    content
        .lines()
        .filter(|line| line.starts_with(ROUND_PREFIX))
        .map(|line| line.split_whitespace().collect())
        .collect()
}

fn negative_lines(content: &str) -> Vec<&str> {
    content
        .lines()
        .filter(|line| line.starts_with(NEGATIVE_PREFIX))
        .collect()
}

fn shell_waitset_or_composition_roundtrip(session: &mut QemuSession) {
    let mark = session.mark();
    session.send_line("waitset");

    if let Err(output_tail) =
        session.wait_until_since(mark, |delta| delta.contains(LAST_LINE), SWEEP_TIMEOUT)
    {
        panic!(
            "timed out after {SWEEP_TIMEOUT:?} waiting for {LAST_LINE:?}; output tail:\n{output_tail}"
        );
    }

    let content = session.content_since(mark);

    let rounds = round_tokens(&content);
    assert_eq!(
        rounds.len(),
        3,
        "expected exactly three lines starting with {ROUND_PREFIX:?}\n{}",
        window_diagnostics(&content)
    );
    for (index, tokens) in rounds.iter().enumerate() {
        let raw_line = tokens.join(" ");
        assert_eq!(
            tokens.len(),
            6,
            "round line {index} must split into exactly 6 tokens, got {tokens:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[0], "waitset",
            "round line {index} must start with waitset, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[1], "round:",
            "round line {index} must carry round: marker, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[2],
            (index + 1).to_string().as_str(),
            "round line {index} must announce round {}, got {raw_line:?}\n{}",
            index + 1,
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[3], "tag:",
            "round line {index} must carry tag: marker, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[4], EXPECTED_TAGS[index],
            "round line {index} must carry tag {}, got {raw_line:?}\n{}",
            EXPECTED_TAGS[index],
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[5], "ok",
            "round line {index} verdict must be ok, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
    }

    let negatives = negative_lines(&content);
    assert_eq!(
        negatives.len(),
        5,
        "expected exactly five lines starting with {NEGATIVE_PREFIX:?}\n{}",
        window_diagnostics(&content)
    );
    for (index, expected) in EXPECTED_NEGATIVE_LINES.iter().enumerate() {
        assert_eq!(
            negatives[index], *expected,
            "negative line {index} must be {expected:?} in plan order\n{}",
            window_diagnostics(&content)
        );
    }

    assert!(
        content.contains(LAST_LINE),
        "window must contain the final sweep line {LAST_LINE:?}\n{}",
        window_diagnostics(&content)
    );

    for forbidden in FORBIDDEN_STRINGS {
        assert!(
            !content.contains(forbidden),
            "window contains forbidden string {forbidden:?}\n{}",
            window_diagnostics(&content)
        );
    }
}
