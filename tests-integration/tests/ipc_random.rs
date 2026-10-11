use std::time::Duration;
use tests_integration::qemu::QemuSession;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const ITERATIONS: usize = 259;
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;
const SERVER_PREFIX: &str = "RANDOM Server: ";
const VALUE_REQUESTED: &str = "RANDOM Value requested";
const VALUE_PREFIX: &str = "RANDOM Value: ";
const FRESH_VALUE_PREFIX: &str = "RANDOM Fresh value: ";
const STALE_REJECTED: &str = "RANDOM Stale rejected";
const FAILURE_STRINGS: [&str; 7] = [
    "RANDOM Value not received",
    "RANDOM Failed to send",
    "Connection failed",
    "RANDOM Stale accepted",
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

fn parse_token(raw: &str, line: &str) -> usize {
    usize::from_str_radix(raw.trim(), 16).unwrap_or_else(|err| {
        panic!("cannot parse {raw:?} as hex token from raw line {line:?}: {err}")
    })
}

fn server_tokens(content: &str) -> Vec<usize> {
    content
        .lines()
        .filter(|line| line.starts_with(SERVER_PREFIX))
        .map(|line| parse_token(&line[SERVER_PREFIX.len()..], line))
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
    let tokens = server_tokens(content);
    let parsed_values = values(content);
    let first_packed = tokens.first().copied();
    let max_packed = tokens.iter().max().copied();
    format!(
        "server_count={} requested_count={} value_count={} first_packed={first_packed:?} max_packed={max_packed:?} values_len={} window_tail={}",
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

    let tokens = server_tokens(&content);
    assert_eq!(
        tokens.len(),
        ITERATIONS,
        "unexpected number of parsed server handle tokens\n{}",
        window_diagnostics(&content)
    );
    assert!(
        content.contains(STALE_REJECTED),
        "expected the recycled first connection to be rejected on send\n{}",
        window_diagnostics(&content)
    );
    assert_eq!(
        count_prefixes(&content, FRESH_VALUE_PREFIX),
        1,
        "expected exactly one fresh roundtrip value line\n{}",
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
