use std::time::Duration;
use tests_integration::qemu::QemuSession;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;
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
        tail_chars(content, DIAGNOSTIC_TAIL_CHARS),
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

#[test]
fn shell_waitset_or_composition_roundtrip() {
    let mut session = QemuSession::spawn();

    let mark = session.mark();
    session.expect_output("rose>", READY_TIMEOUT);

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
