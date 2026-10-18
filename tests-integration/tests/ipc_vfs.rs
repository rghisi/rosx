use std::time::Duration;
use tests_integration::qemu::QemuSession;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const SWEEP_TIMEOUT: Duration = Duration::from_secs(180);
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;
const BOOT_LINE: &str = "[IPC Server] VFS";
const PROMPT: &str = "rose>";
const COMMAND: &str = "vfs";
const OK_PREFIX: &str = "vfs ok: ";
const DONE_LINE: &str = "vfs done";
const ERROR_PREFIX: &str = "vfs error:";
const FORBIDDEN_STRINGS: [&str; 2] = ["fail", "Panic"];

const EXPECTED_OK: [&str; 24] = [
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
    "vfs ok: rom list complete",
    "vfs ok: rom hello_elf elf magic",
    "vfs ok: rom write denied",
    "vfs ok: rom create denied",
    "vfs ok: rom delete denied",
];

fn tail_chars(content: &str, n: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= n {
        return content.to_string();
    }
    content.chars().skip(char_count - n).collect()
}

fn window_diagnostics(content: &str) -> String {
    let ok_count = content
        .lines()
        .filter(|line| line.starts_with(OK_PREFIX))
        .count();
    format!(
        "ok_line_count={} window_tail={}",
        ok_count,
        tail_chars(content, DIAGNOSTIC_TAIL_CHARS)
    )
}

#[test]
fn shell_vfs_filesystem_roundtrip() {
    let mut session = QemuSession::spawn();

    let mark = session.mark();
    session.expect_output(BOOT_LINE, READY_TIMEOUT);
    session.expect_output(PROMPT, READY_TIMEOUT);

    session.send_line(COMMAND);

    if let Err(output_tail) =
        session.wait_until_since(mark, |delta| delta.contains(DONE_LINE), SWEEP_TIMEOUT)
    {
        panic!(
            "timed out after {SWEEP_TIMEOUT:?} waiting for {DONE_LINE:?}; output tail:\n{output_tail}"
        );
    }

    let content = session.content_since(mark);
    let lines: Vec<&str> = content.lines().collect();

    let ok_lines: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with(OK_PREFIX))
        .collect();
    assert_eq!(
        ok_lines.len(),
        EXPECTED_OK.len(),
        "expected exactly {} lines starting with {OK_PREFIX:?}\n{}",
        EXPECTED_OK.len(),
        window_diagnostics(&content)
    );
    for (index, expected) in EXPECTED_OK.iter().enumerate() {
        assert_eq!(
            ok_lines[index], *expected,
            "ok line {index} must equal {expected:?}, got {:?}\n{}",
            ok_lines[index],
            window_diagnostics(&content)
        );
    }

    let done_indices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == DONE_LINE)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        done_indices.len(),
        1,
        "expected exactly one line equal to {DONE_LINE:?}\n{}",
        window_diagnostics(&content)
    );
    let last_ok_index = lines
        .iter()
        .rposition(|line| line.starts_with(OK_PREFIX))
        .expect("at least one ok line precedes the done line");
    assert!(
        done_indices[0] > last_ok_index,
        "the {DONE_LINE:?} line must follow the last ok line\n{}",
        window_diagnostics(&content)
    );

    assert!(
        lines.iter().all(|line| !line.starts_with(ERROR_PREFIX)),
        "window contains a {ERROR_PREFIX:?} line\n{}",
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
