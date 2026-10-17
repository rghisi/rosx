use std::time::Duration;
use tests_integration::qemu::QemuSession;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const PING_SIZES: [usize; 3] = [64, 4096, 65536];
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;
const CLIENT_PREFIX: &str = "ping sent: ";
const SERVER_PREFIX: &str = "PONG ";
const SERVER_MATCHED: &str = " matched ";
const LAST_SWEEP_LINE: &str = "ping sent: 65536";
const FORBIDDEN_STRINGS: [&str; 2] = ["fail", "Panic"];

fn count_prefixes(content: &str, prefix: &str) -> usize {
    content
        .lines()
        .filter(|line| line.starts_with(prefix))
        .count()
}

fn count_server_rounds(content: &str) -> usize {
    content
        .lines()
        .filter(|line| line.starts_with(SERVER_PREFIX) && line.contains(SERVER_MATCHED))
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
        "client_line_count={} server_line_count={} window_tail={}",
        count_prefixes(content, CLIENT_PREFIX),
        count_server_rounds(content),
        tail_chars(content, DIAGNOSTIC_TAIL_CHARS),
    )
}

fn client_round_tokens(content: &str) -> Vec<Vec<&str>> {
    content
        .lines()
        .filter(|line| line.starts_with(CLIENT_PREFIX))
        .map(|line| line.split_whitespace().collect())
        .collect()
}

fn server_round_tokens(content: &str) -> Vec<Vec<&str>> {
    content
        .lines()
        .filter(|line| line.starts_with(SERVER_PREFIX) && line.contains(SERVER_MATCHED))
        .map(|line| line.split_whitespace().collect())
        .collect()
}

fn parse_size(raw: &str, line: &str) -> usize {
    raw.parse::<usize>()
        .unwrap_or_else(|err| panic!("cannot parse {raw:?} as usize from raw line {line:?}: {err}"))
}

#[test]
fn shell_ping_capability_data_roundtrip() {
    let mut session = QemuSession::spawn();

    let mark = session.mark();
    session.expect_output("[IPC Server] Ping", READY_TIMEOUT);
    session.expect_output("rose>", READY_TIMEOUT);

    session.send_line("ping");

    if let Err(output_tail) =
        session.wait_until_since(mark, |delta| delta.contains(LAST_SWEEP_LINE), SWEEP_TIMEOUT)
    {
        panic!(
            "timed out after {SWEEP_TIMEOUT:?} waiting for {LAST_SWEEP_LINE:?}; output tail:\n{output_tail}"
        );
    }

    let content = session.content_since(mark);

    let client = client_round_tokens(&content);
    assert_eq!(
        client.len(),
        PING_SIZES.len(),
        "expected exactly three lines starting with {CLIENT_PREFIX:?}\n{}",
        window_diagnostics(&content)
    );
    for (index, tokens) in client.iter().enumerate() {
        let raw_line = tokens.join(" ");
        assert_eq!(
            tokens.len(),
            7,
            "client line {index} must split into exactly 7 tokens, got {tokens:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[0],
            "ping",
            "client line {index} must start with ping, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[1],
            "sent:",
            "client line {index} must carry sent: marker, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[4],
            "received:",
            "client line {index} must carry received: marker, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
        let sent_size = parse_size(tokens[2], &raw_line);
        let received_size = parse_size(tokens[5], &raw_line);
        assert_eq!(
            sent_size,
            PING_SIZES[index],
            "client line {index} must announce size {}\n{}",
            PING_SIZES[index],
            window_diagnostics(&content)
        );
        assert_eq!(
            received_size,
            PING_SIZES[index],
            "client line {index} must report received size {}\n{}",
            PING_SIZES[index],
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[3],
            "ok",
            "client line {index} sent verdict must be ok, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[6],
            "ok",
            "client line {index} received verdict must be ok, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
    }

    let server = server_round_tokens(&content);
    assert_eq!(
        server.len(),
        PING_SIZES.len(),
        "expected exactly three server PONG round lines\n{}",
        window_diagnostics(&content)
    );
    for (index, tokens) in server.iter().enumerate() {
        let raw_line = tokens.join(" ");
        assert_eq!(
            tokens.len(),
            4,
            "server line {index} must split into exactly 4 tokens, got {tokens:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[0],
            "PONG",
            "server line {index} must start with PONG, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
        let pong_size = parse_size(tokens[1], &raw_line);
        assert_eq!(
            pong_size,
            PING_SIZES[index],
            "server line {index} must echo size {}\n{}",
            PING_SIZES[index],
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[2],
            "matched",
            "server line {index} must carry matched marker, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
        assert_eq!(
            tokens[3],
            "ok",
            "server line {index} verdict must be ok, got {raw_line:?}\n{}",
            window_diagnostics(&content)
        );
    }

    for forbidden in FORBIDDEN_STRINGS {
        assert!(
            !content.contains(forbidden),
            "window contains forbidden string {forbidden:?}\n{}",
            window_diagnostics(&content)
        );
    }
}
