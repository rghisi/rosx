use std::path::Path;
use std::time::Duration;
use tests_integration::monitor::Monitor;
use tests_integration::output::strip_ansi;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const MONITOR_STEP_TIMEOUT: Duration = Duration::from_secs(10);

const MIRROR_STATUS_PORT: u16 = 0x3F6;
const COMMAND_PORT: u16 = 0x1F7;
const DRIVE_HEAD_PORT: u16 = 0x1F6;
const DATA_PORT: u16 = 0x1F0;

const SOFT_RESET_ASSERT: u8 = 0x04;
const SOFT_RESET_DEASSERT: u8 = 0x00;
const SELECT_DRIVE_0_LBA_MODE_OFF: u8 = 0xA0;
const IDENTIFY_COMMAND: u8 = 0xEC;

const STATUS_BUSY: u8 = 0x80;
const STATUS_READY: u8 = 0x40;
const STATUS_FAULT: u8 = 0x01;
const STATUS_DATA_REQUEST: u8 = 0x08;

const IDENTIFY_MODEL_FIRST_WORD: usize = 27;
const IDENTIFY_MODEL_LAST_WORD: usize = 46;
const IDENTIFY_WORD_COUNT: usize = 256;
const IDENTIFY_DWORD_COUNT: usize = IDENTIFY_WORD_COUNT / 2;

const STATUS_POLL_ATTEMPTS: usize = 50;

fn parse_monitor_value(reply: &str, command: &str) -> u64 {
    let clean = strip_ansi(reply);
    let marker = clean
        .find(" = 0x")
        .unwrap_or_else(|| panic!("monitor reply carries no value for {command:?}\nreply: {clean:?}"));
    let digits_start = marker + " = 0x".len();
    let digits_end = clean[digits_start..]
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(clean[digits_start..].len());
    u64::from_str_radix(&clean[digits_start..digits_start + digits_end], 16)
        .unwrap_or_else(|err| panic!("cannot parse value of {command:?}: {err}"))
}

fn write_byte(monitor: &mut Monitor, port: u16, value: u8) {
    let command = format!("o /b {port:#x} {value:#x}");
    let reply = monitor
        .send_command(&command)
        .unwrap_or_else(|err| panic!("monitor write {command:?} failed: {err}"));
    let clean = strip_ansi(&reply);
    if clean.contains("invalid") || clean.contains("error") {
        panic!("monitor rejected {command:?}: {clean:?}");
    }
}

fn read_byte(monitor: &mut Monitor, port: u16) -> u8 {
    let command = format!("i /b {port:#x}");
    let reply = monitor
        .send_command(&command)
        .unwrap_or_else(|err| panic!("monitor read {command:?} failed: {err}"));
    parse_monitor_value(&reply, &command) as u8
}

fn read_dword(monitor: &mut Monitor, port: u16) -> u32 {
    let command = format!("i /w {port:#x}");
    let reply = monitor
        .send_command(&command)
        .unwrap_or_else(|err| panic!("monitor read {command:?} failed: {err}"));
    parse_monitor_value(&reply, &command) as u32
}

fn monitor_of(session: &QemuSession) -> Monitor {
    let socket: &Path = &session.temp_dir().join("monitor.sock");
    Monitor::connect(socket).unwrap_or_else(|err| panic!("cannot reach monitor socket: {err}"))
}

fn decode_identify_model(words: &[u16]) -> String {
    let mut raw = Vec::new();
    for word in &words[IDENTIFY_MODEL_FIRST_WORD..=IDENTIFY_MODEL_LAST_WORD] {
        raw.push((word >> 8) as u8);
        raw.push((word & 0xFF) as u8);
    }
    String::from_utf8_lossy(&raw).trim_end().to_string()
}

#[test]
fn qemu_exposes_the_boot_disk_on_the_primary_ide_channel() {
    if tests_integration::arch::not_ported_to_x86_32() {
        return;
    }
    let session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);

    let mut monitor = monitor_of(&session);

    write_byte(&mut monitor, MIRROR_STATUS_PORT, SOFT_RESET_ASSERT);
    write_byte(&mut monitor, MIRROR_STATUS_PORT, SOFT_RESET_DEASSERT);

    let floated = read_byte(&mut monitor, COMMAND_PORT);
    assert_ne!(floated, 0xFF, "compat task file floats — no IDE device on 0x1F0");

    write_byte(&mut monitor, DRIVE_HEAD_PORT, SELECT_DRIVE_0_LBA_MODE_OFF);
    write_byte(&mut monitor, COMMAND_PORT, IDENTIFY_COMMAND);

    let mut staged = false;
    let mut last_status = 0u8;
    for _ in 0..STATUS_POLL_ATTEMPTS {
        last_status = read_byte(&mut monitor, COMMAND_PORT);
        assert_eq!(
            last_status & STATUS_FAULT,
            0,
            "IDENTIFY faulted with error register follow-up status {last_status:#04x}"
        );
        if last_status & STATUS_BUSY == 0 && last_status & STATUS_DATA_REQUEST != 0 {
            staged = true;
            break;
        }
    }
    assert!(
        staged,
        "DRQ never asserted after IDENTIFY (last status {last_status:#04x})"
    );
    assert_ne!(last_status & STATUS_READY, 0, "device not ready after IDENTIFY");

    let mut words = vec![0u16; IDENTIFY_WORD_COUNT];
    for dword_index in 0..IDENTIFY_DWORD_COUNT {
        let dword = read_dword(&mut monitor, DATA_PORT);
        words[dword_index * 2] = (dword & 0xFFFF) as u16;
        words[dword_index * 2 + 1] = (dword >> 16) as u16;
    }

    let model = decode_identify_model(&words);
    assert!(
        model.contains("HARDDISK"),
        "IDENTIFY model string {model:?} does not name the QEMU hard disk"
    );

    let _ = monitor.send_command("quit");
}
