use std::time::Duration;
use tests_integration::qemu::QemuSession;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const ITERATIONS: usize = 259;
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;
const SERVER_PREFIX: &str = "RANDOM Server: ";
const VALUE_REQUESTED: &str = "RANDOM Value requested";
const VALUE_PREFIX: &str = "RANDOM Value: ";
const FAILURE_STRINGS: [&str; 6] = [
    "RANDOM Value not received",
    "RANDOM Failed to send",
    "Connection failed",
    "[IPC Server] Random - Panic!",
    "Buffer pool exhausted",
    "Buffer not found",
];

fn count_prefixes(content: &str, prefix: &str) -> usize {
    content
        .lines()
        .filter(|line| line.starts_with(prefix))
        .count()
}

fn parse_number(raw: &str, line: &str) -> usize {
    raw.trim()
        .parse::<usize>()
        .unwrap_or_else(|err| panic!("cannot parse {raw:?} as usize from raw line {line:?}: {err}"))
}

fn server_pairs(content: &str) -> Vec<(usize, usize)> {
    content
        .lines()
        .filter(|line| line.starts_with(SERVER_PREFIX))
        .map(|line| {
            let rest = line[SERVER_PREFIX.len()..].trim();
            let (index, generation) = rest
                .split_once(' ')
                .unwrap_or_else(|| panic!("malformed server line {line:?}"));
            (parse_number(index, line), parse_number(generation, line))
        })
        .collect()
}

fn values(content: &str) -> Vec<usize> {
    content
        .lines()
        .filter(|line| line.starts_with(VALUE_PREFIX))
        .map(|line| parse_number(&line[VALUE_PREFIX.len()..], line))
        .collect()
}

fn tail_chars(content: &str, n: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= n {
        return content.to_string();
    }
    content.chars().skip(char_count - n).collect()
}

fn window_diagnostics(content: &str) -> String {
    let pairs = server_pairs(content);
    let parsed_values = values(content);
    let first_generation = pairs.first().map(|(_, generation)| *generation);
    let max_generation = pairs.iter().map(|(_, generation)| *generation).max();
    format!(
        "server_count={} requested_count={} value_count={} first_generation={first_generation:?} max_generation={max_generation:?} values_len={} window_tail={}",
        count_prefixes(content, SERVER_PREFIX),
        count_prefixes(content, VALUE_REQUESTED),
        count_prefixes(content, VALUE_PREFIX),
        parsed_values.len(),
        tail_chars(content, DIAGNOSTIC_TAIL_CHARS),
    )
}

#[test]
fn shell_random_capability_ipc_roundtrip() {
    let mut session = QemuSession::spawn();
    session.expect_output("[IPC Server] Random", READY_TIMEOUT);
    session.expect_output("rose>", READY_TIMEOUT);

    let mark = session.mark();
    session.send_line("random");

    if let Err(output_tail) =
        session.wait_until_since(mark, |delta| delta.contains("rose>"), SWEEP_TIMEOUT)
    {
        panic!(
            "timed out after {SWEEP_TIMEOUT:?} waiting for the random sweep to complete; output tail:\n{output_tail}"
        );
    }

    let content = session.content_since(mark);

    assert_eq!(
        count_prefixes(&content, SERVER_PREFIX),
        ITERATIONS,
        "unexpected count of lines starting with {SERVER_PREFIX:?}\n{}",
        window_diagnostics(&content)
    );
    assert_eq!(
        count_prefixes(&content, VALUE_REQUESTED),
        ITERATIONS,
        "unexpected count of lines starting with {VALUE_REQUESTED:?}\n{}",
        window_diagnostics(&content)
    );
    assert_eq!(
        count_prefixes(&content, VALUE_PREFIX),
        ITERATIONS,
        "unexpected count of lines starting with {VALUE_PREFIX:?}\n{}",
        window_diagnostics(&content)
    );

    let pairs = server_pairs(&content);
    assert_eq!(
        pairs.len(),
        ITERATIONS,
        "unexpected number of parsed server handle pairs\n{}",
        window_diagnostics(&content)
    );
    let first_generation = pairs[0].1;
    assert!(
        pairs
            .iter()
            .any(|(_, generation)| *generation > first_generation),
        "expected at least one connection generation above the first observed {first_generation}\n{}",
        window_diagnostics(&content)
    );

    let parsed_values = values(&content);
    assert_eq!(
        parsed_values.len(),
        ITERATIONS,
        "unexpected number of parsed random values\n{}",
        window_diagnostics(&content)
    );
    assert!(
        parsed_values
            .windows(2)
            .all(|window| window[0] != window[1]),
        "consecutive random values must differ\n{}",
        window_diagnostics(&content)
    );

    for failure in FAILURE_STRINGS {
        assert!(
            !content.contains(failure),
            "window contains forbidden string {failure:?}\n{}",
            window_diagnostics(&content)
        );
    }
}
