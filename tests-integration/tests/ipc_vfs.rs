use std::time::Duration;
use tests_integration::qemu::QemuSession;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;
const OK_PREFIX: &str = "vfs ok: ";
const ERROR_PREFIX: &str = "vfs error:";
const DONE_LINE: &str = "vfs done";
const FORBIDDEN_STRINGS: [&str; 2] = ["fail", "Panic"];
const EXPECTED_OK: [&str; 19] = [
    "vfs ok: create notes.txt",
    "vfs ok: write 4096 at 0",
    "vfs ok: write 1024 at 8192",
    "vfs ok: read 9216 hole zeroed",
    "vfs ok: eof at 9216",
    "vfs ok: zero write accepted",
    "vfs ok: write denied mode 0",
    "vfs ok: reopen kept 9216",
    "vfs ok: list got notes.txt",
    "vfs ok: create existing denied",
    "vfs ok: open missing denied",
    "vfs ok: chunked write 131072",
    "vfs ok: chunked read 131072 verified",
    "vfs ok: list got blob.bin notes.txt",
    "vfs ok: delete notes.txt",
    "vfs ok: delete twice denied",
    "vfs ok: list got blob.bin",
    "vfs ok: delete blob.bin",
    "vfs ok: list empty",
];

fn ok_lines(content: &str) -> Vec<&str> {
    content
        .lines()
        .filter(|line| line.starts_with(OK_PREFIX))
        .collect()
}

fn count_prefix_lines(content: &str, prefix: &str) -> usize {
    content
        .lines()
        .filter(|line| line.starts_with(prefix))
        .count()
}

fn count_exact_lines(content: &str, target: &str) -> usize {
    content.lines().filter(|line| *line == target).count()
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
        "ok_line_count={} error_line_count={} done_line_count={} window_tail={}",
        count_prefix_lines(content, OK_PREFIX),
        count_prefix_lines(content, ERROR_PREFIX),
        count_exact_lines(content, DONE_LINE),
        tail_chars(content, DIAGNOSTIC_TAIL_CHARS),
    )
}

#[test]
fn shell_vfs_filesystem_roundtrip() {
    let mut session = QemuSession::spawn();

    let mark = session.mark();
    session.expect_output("[IPC Server] VFS", READY_TIMEOUT);
    session.expect_output("rose>", READY_TIMEOUT);

    session.send_line("vfs");

    if let Err(output_tail) =
        session.wait_until_since(mark, |delta| delta.contains(DONE_LINE), SWEEP_TIMEOUT)
    {
        panic!(
            "timed out after {SWEEP_TIMEOUT:?} waiting for {DONE_LINE:?}; output tail:\n{output_tail}"
        );
    }

    let content = session.content_since(mark);

    let got_ok = ok_lines(&content);
    assert_eq!(
        got_ok,
        EXPECTED_OK,
        "expected exactly the 19 vfs ok lines in scenario order\n{}",
        window_diagnostics(&content)
    );

    assert_eq!(
        count_exact_lines(&content, DONE_LINE),
        1,
        "expected exactly one line equal to {DONE_LINE:?}\n{}",
        window_diagnostics(&content)
    );

    let lines: Vec<&str> = content.lines().collect();
    let last_ok_index = lines
        .iter()
        .copied()
        .rposition(|line| line.starts_with(OK_PREFIX))
        .expect("ok lines are proven non-empty by the element-wise equality assertion above");
    let done_index = lines
        .iter()
        .copied()
        .rposition(|line| line == DONE_LINE)
        .expect("done line is proven unique by the count assertion above");
    assert!(
        done_index > last_ok_index,
        "{DONE_LINE:?} must follow the last ok line, ok_index={last_ok_index} done_index={done_index}\n{}",
        window_diagnostics(&content)
    );

    assert_eq!(
        count_prefix_lines(&content, ERROR_PREFIX),
        0,
        "no line may start with {ERROR_PREFIX:?}\n{}",
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
