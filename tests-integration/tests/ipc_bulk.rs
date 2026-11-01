use std::time::Duration;
use tests_integration::qemu::QemuSession;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;
const OBJECT_LENGTHS: [usize; 3] = [32768, 32768, 8192];
const TOTAL_BYTES: usize = 73728;
const ECHO_PREFIX: &str = "BULK Echo: ";
const TOOLARGE_REJECTED: &str = "BULK TooLarge rejected";
const BUDGET_REJECTED: &str = "BULK Budget rejected";
const REFUND_OK: &str = "BULK Refund ok";
const VERIFIED_LINE: &str = "BULK Verified: 73728";
const DONE_LINE: &str = "BULK Done";
const FAILURE_STRINGS: [&str; 4] = [
    "BULK Mismatch",
    "BULK Connection failed",
    "[IPC Server] BulkEcho - Panic!",
    "[BULK Client] Panic!",
];

fn expected_checksum(object_index: usize, length: usize) -> u32 {
    (0..length).fold(0u32, |sum, byte_index| {
        sum.wrapping_add(((byte_index * 7 + object_index) as u8) as u32)
    })
}

fn echo_triples(content: &str) -> Vec<(usize, u32)> {
    content
        .lines()
        .filter(|line| line.contains(ECHO_PREFIX))
        .map(|line| {
            let rest = line
                .split_once(ECHO_PREFIX)
                .map(|(_head, tail)| tail)
                .unwrap_or(line);
            let mut fields = rest.split_whitespace();
            let bytes_raw = fields
                .next()
                .unwrap_or_else(|| panic!("missing byte count in raw line {line:?}"));
            let checksum_raw = fields
                .next()
                .unwrap_or_else(|| panic!("missing checksum in raw line {line:?}"));
            let bytes = bytes_raw.parse::<usize>().unwrap_or_else(|err| {
                panic!("cannot parse {bytes_raw:?} as usize from raw line {line:?}: {err}")
            });
            let checksum = checksum_raw.parse::<u32>().unwrap_or_else(|err| {
                panic!("cannot parse {checksum_raw:?} as u32 from raw line {line:?}: {err}")
            });
            (bytes, checksum)
        })
        .collect()
}

fn count_contained(content: &str, needle: &str) -> usize {
    content.lines().filter(|line| line.contains(needle)).count()
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
        "echo_count={} toolarge_count={} budget_count={} refund_count={} verified_count={} done_count={} window_tail={}",
        count_contained(content, ECHO_PREFIX),
        count_contained(content, TOOLARGE_REJECTED),
        count_contained(content, BUDGET_REJECTED),
        count_contained(content, REFUND_OK),
        count_contained(content, VERIFIED_LINE),
        count_contained(content, DONE_LINE),
        tail_chars(content, DIAGNOSTIC_TAIL_CHARS),
    )
}

#[test]
fn bulk_data_transfer_echo_and_limits_roundtrip() {
    if tests_integration::arch::not_ported_to_x86_32() {
        return;
    }
    let session = QemuSession::spawn();

    let mark = session.mark();
    session.expect_output("[IPC Server] BulkEcho", READY_TIMEOUT);
    session.expect_output("rose>", READY_TIMEOUT);

    if let Err(output_tail) =
        session.wait_until_since(mark, |delta| delta.contains(DONE_LINE), SWEEP_TIMEOUT)
    {
        panic!(
            "timed out after {SWEEP_TIMEOUT:?} waiting for {DONE_LINE:?}; output tail:\n{output_tail}"
        );
    }

    let content = session.content_since(mark);

    let mut got_triples = echo_triples(&content);
    got_triples.sort();
    let mut expected_triples: Vec<(usize, u32)> = OBJECT_LENGTHS
        .iter()
        .enumerate()
        .map(|(object_index, length)| (*length, expected_checksum(object_index, *length)))
        .collect();
    expected_triples.sort();
    assert_eq!(
        got_triples,
        expected_triples,
        "echo triples must byte-equal the host-recomputed checksums\n{}",
        window_diagnostics(&content)
    );

    for marker in [
        TOOLARGE_REJECTED,
        BUDGET_REJECTED,
        REFUND_OK,
        VERIFIED_LINE,
        DONE_LINE,
    ] {
        assert_eq!(
            count_contained(&content, marker),
            1,
            "expected exactly one line containing {marker:?}\n{}",
            window_diagnostics(&content)
        );
    }
    assert_eq!(
        TOTAL_BYTES,
        OBJECT_LENGTHS.iter().sum::<usize>(),
        "length constants must stay consistent"
    );

    for failure in FAILURE_STRINGS {
        assert!(
            !content.contains(failure),
            "window contains forbidden string {failure:?}\n{}",
            window_diagnostics(&content)
        );
    }
}
