use std::time::Duration;

use tests_integration::fat_disk;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const KEY_ATTEMPTS: usize = 4;
const KEY_ACK_BUDGET: Duration = Duration::from_secs(6);

fn session_at_prompt() -> QemuSession {
    let session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);
    session
}

fn send_key_awaited(session: &mut QemuSession, key: char, needle: &str) {
    for attempt in 1..=KEY_ATTEMPTS {
        let mark = session.mark();
        session.send_key(key);
        if session
            .wait_until_since(mark, |delta| delta.contains(needle), KEY_ACK_BUDGET)
            .is_ok()
        {
            return;
        }
        if attempt == KEY_ATTEMPTS {
            panic!("keystroke {key:?} never produced {needle:?} after {KEY_ATTEMPTS} attempts");
        }
    }
}

fn launch_ted(session: &mut QemuSession, arguments: &str) {
    session.send_text(arguments);
    send_key_awaited(session, '\n', "^S Save");
}

fn saved_notice(session: &mut QemuSession, message: &str) {
    send_key_awaited(session, '\x13', message);
}

fn quit_to_shell(session: &mut QemuSession) {
    send_key_awaited(session, '\x11', "rose>");
}

#[test]
fn ted_creates_saves_and_exits_with_clean_buffer() {
    let mut session = session_at_prompt();
    launch_ted(&mut session, "ted /hello.txt");

    session.send_text("Hello ted");
    saved_notice(&mut session, "saved 10 bytes");
    quit_to_shell(&mut session);

    session.shutdown();
    let image = session.temp_dir().join("boot-disk.img");
    assert_eq!(
        fat_disk::read_file_from_image(&image, "hello.txt"),
        b"Hello ted\n"
    );
}

#[test]
fn ted_shrink_save_rewrites_the_shorter_file() {
    let mut session = session_at_prompt();
    launch_ted(&mut session, "ted /shrink.txt");

    session.send_text("abcdefgh");
    saved_notice(&mut session, "saved 9 bytes");
    send_key_awaited(&mut session, '\x08', "Col 8");
    send_key_awaited(&mut session, '\x08', "Col 7");
    send_key_awaited(&mut session, '\x08', "Col 6");
    saved_notice(&mut session, "saved 6 bytes");
    quit_to_shell(&mut session);

    session.shutdown();
    let image = session.temp_dir().join("boot-disk.img");
    assert_eq!(
        fat_disk::read_file_from_image(&image, "shrink.txt"),
        b"abcde\n"
    );
}

#[test]
fn ted_quit_prompt_answers_no_then_yes() {
    let mut session = session_at_prompt();
    launch_ted(&mut session, "ted /guard.txt");

    session.send_text("dirt");
    send_key_awaited(&mut session, '\x11', "Save modified buffer?");
    send_key_awaited(&mut session, 'n', "Col 5");

    send_key_awaited(&mut session, '\x11', "Save modified buffer?");
    send_key_awaited(&mut session, 'y', "rose>");

    session.shutdown();
    let image = session.temp_dir().join("boot-disk.img");
    assert_eq!(
        fat_disk::read_file_from_image(&image, "guard.txt"),
        b"dirt\n"
    );
}
