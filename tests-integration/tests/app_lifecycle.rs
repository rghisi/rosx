use std::thread;
use std::time::{Duration, Instant};
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const BANNER_TIMEOUT: Duration = Duration::from_secs(30);
const REPAINT_TIMEOUT: Duration = Duration::from_secs(60);
const REPAINT_POLL_INTERVAL: Duration = Duration::from_millis(250);
const QUIT_TIMEOUT: Duration = Duration::from_secs(20);
const TETRIS_IDLE_SLEEP: Duration = Duration::from_secs(45);
const SNAKE_IDLE_SLEEP: Duration = Duration::from_secs(35);
const MIN_TETRIS_REPAINTS: usize = 3;
const DIAGNOSTIC_TAIL_CHARS: usize = 2000;

#[test]
fn tetris_survives_idle() {
    let booted_at = Instant::now();
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);
    let mark = session.mark();
    session.send_line("tetris");
    session.expect_output_since(mark, "TETRIS", BANNER_TIMEOUT);

    let repaint_deadline = Instant::now() + REPAINT_TIMEOUT;
    loop {
        assert!(
            session.is_running(),
            "tetris: QEMU exited {:.1?} after spawn while waiting for {MIN_TETRIS_REPAINTS} TETRIS repaints\ncaptured output tail:\n{}",
            booted_at.elapsed(),
            tail_chars(&session.content_since(mark), DIAGNOSTIC_TAIL_CHARS),
        );
        let content = session.content_since(mark);
        if content.matches("TETRIS").count() >= MIN_TETRIS_REPAINTS {
            break;
        }
        assert!(
            Instant::now() < repaint_deadline,
            "tetris animation stalled: fewer than {MIN_TETRIS_REPAINTS} TETRIS repaints within {REPAINT_TIMEOUT:?}\ncaptured output tail:\n{}",
            tail_chars(&content, DIAGNOSTIC_TAIL_CHARS),
        );
        thread::sleep(REPAINT_POLL_INTERVAL);
    }

    thread::sleep(TETRIS_IDLE_SLEEP);
    assert!(
        session.is_running(),
        "tetris triple faulted: QEMU exited {:.1?} after spawn while guest idled past the fault horizon\ncaptured output tail:\n{}",
        booted_at.elapsed(),
        tail_chars(&session.content_since(mark), DIAGNOSTIC_TAIL_CHARS),
    );

    session.send_key('q');
    session.expect_output_since(mark, "rose>", QUIT_TIMEOUT);
}

#[test]
fn snake_survives_idle() {
    let booted_at = Instant::now();
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);
    let mark = session.mark();
    session.send_line("snake");
    session.expect_output_since(mark, "SNAKE", BANNER_TIMEOUT);

    let game_over_deadline = Instant::now() + BANNER_TIMEOUT;
    while !session.content_since(mark).contains("GAME OVER") {
        assert!(
            session.is_running(),
            "snake: QEMU exited {:.1?} after spawn before rendering GAME OVER\ncaptured output tail:\n{}",
            booted_at.elapsed(),
            tail_chars(&session.content_since(mark), DIAGNOSTIC_TAIL_CHARS),
        );
        assert!(
            Instant::now() < game_over_deadline,
            "snake: GAME OVER not rendered within {BANNER_TIMEOUT:?} while QEMU still ran\ncaptured output tail:\n{}",
            tail_chars(&session.content_since(mark), DIAGNOSTIC_TAIL_CHARS),
        );
        thread::sleep(REPAINT_POLL_INTERVAL);
    }

    thread::sleep(SNAKE_IDLE_SLEEP);
    assert!(
        session.is_running(),
        "snake triple faulted: QEMU exited {:.1?} after spawn while guest idled past the fault horizon\ncaptured output tail:\n{}",
        booted_at.elapsed(),
        tail_chars(&session.content_since(mark), DIAGNOSTIC_TAIL_CHARS),
    );

    session.send_key('q');
    session.expect_output_since(mark, "Goodbye", QUIT_TIMEOUT);
}

fn tail_chars(content: &str, n: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= n {
        return content.to_string();
    }
    content.chars().skip(char_count - n).collect()
}
