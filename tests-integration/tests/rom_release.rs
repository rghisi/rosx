use std::time::Duration;
use tests_integration::arch::Arch;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const OP_BUDGET: Duration = Duration::from_secs(60);
const RELEASE_MARKER: &str = "[ROM] released addr=";
const PAGE_BYTES: u64 = 4096;
const FREE_SLACK_KIB: u64 = 24;

fn line_at(transcript: &str, marker: &str) -> String {
    transcript
        .split(marker)
        .nth(1)
        .unwrap_or_else(|| panic!("the transcript carries {marker:?}"))
        .lines()
        .next()
        .expect("the marker sits on a line")
        .to_string()
}

fn first_hex(line: &str) -> u64 {
    line.split_whitespace()
        .next()
        .expect("the line opens with a hex field")
        .trim_end_matches(',')
        .strip_prefix("0x")
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .expect("the leading field parses as a hex number")
}

fn decimal_field(line: &str, prefix: &str) -> u64 {
    let after = line
        .split(prefix)
        .nth(1)
        .unwrap_or_else(|| panic!("the line carries {prefix:?}"));
    after
        .split_whitespace()
        .next()
        .expect("the field carries a value")
        .parse()
        .expect("the field is a number")
}

fn count_of(transcript: &str, marker: &str) -> usize {
    transcript.match_indices(marker).count()
}

fn free_kib(session: &mut QemuSession) -> u64 {
    let mark = session.mark();
    session.send_line("mem");
    session.expect_output_since(mark, "uptime:", OP_BUDGET);
    let report = session.content_since(mark);
    let line = report
        .lines()
        .find(|line| line.trim_start().starts_with("free:"))
        .expect("the snapshot carries a free line");
    line.split_whitespace()
        .nth(1)
        .expect("the free line carries a KiB count")
        .parse()
        .expect("the free count is a number")
}

#[test]
fn rom_is_released_once_after_the_boot_plan() {
    let mut session = QemuSession::spawn();
    let start = session.mark();
    session.expect_output("rose>", BOOT_BUDGET);
    let boot = session.content_since(start);

    let rom_line = line_at(&boot, "[ROM] addr=");
    let boot_addr = first_hex(&rom_line);
    let boot_size = decimal_field(&rom_line, "size=");
    assert_eq!(count_of(&boot, RELEASE_MARKER), 1, "the ROM is released exactly once per boot");
    let release_line = line_at(&boot, RELEASE_MARKER);
    let release_addr = first_hex(&release_line);
    let release_bytes = decimal_field(&release_line, "bytes=");
    assert_eq!(
        release_addr, boot_addr,
        "the release returns the very region the boot handed over: {release_line} vs {rom_line}"
    );
    assert!(
        release_bytes >= boot_size && release_bytes < boot_size + PAGE_BYTES,
        "the release size is the ROM image page-rounded: ROM {boot_size}, released {release_bytes}"
    );

    let last_init = boot
        .rfind("[INIT]")
        .expect("the boot plan logs its steps");
    let release_at = boot
        .find(RELEASE_MARKER)
        .expect("the ROM is released");
    let prompt = boot.find("rose>").expect("the shell reaches its prompt");
    assert!(
        release_at > last_init,
        "the release follows every boot-plan step: release at {release_at}, last [INIT] at {last_init}"
    );
    assert!(
        release_at < prompt,
        "the release happens before the shell prompt: release at {release_at}, prompt at {prompt}"
    );
    assert!(!boot.contains("BAD-FREE"), "the release kept the allocator's validation quiet");
    assert!(!boot.contains("[IRQGUARD]"), "the IRQ guard stayed silent");

    let free = free_kib(&mut session);
    let pinned_free = match Arch::current() {
        Arch::X86_64 => 127898u64,
        Arch::X86_32 => 128224,
    };
    println!("free {free} KiB, released {} KiB", release_bytes / 1024);
    assert!(
        free + FREE_SLACK_KIB >= pinned_free,
        "the held ROM came back to the heap: free {free} KiB, S3 release-boot pinned \
         {pinned_free} KiB; a release that never happened leaves free {} KiB lower",
        release_bytes / 1024
    );
}
