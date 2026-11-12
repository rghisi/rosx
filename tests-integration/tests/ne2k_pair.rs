use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const STACK_BUDGET: Duration = Duration::from_secs(45);
const FRAME_BUDGET: Duration = Duration::from_secs(30);

const MULTICAST: &str = "230.0.0.1:19417";

const DESTINATION: &str = "deadbeefcafe";
const SOURCE: &str = "020000000001";
const ETHER_TYPE: &str = "0800";
const FILLER: &str = "52";

fn sample_frame() -> String {
    let mut hex = format!("{DESTINATION}{SOURCE}{ETHER_TYPE}");
    while hex.len() < 120 {
        hex.push_str(FILLER);
    }
    hex
}

#[test]
fn frame_crosses_the_wire_between_two_ne2k_isa_guests() {
    let mut receiver = QemuSession::spawn_ne2k_isa(MULTICAST);
    receiver.expect_output("rose>", BOOT_BUDGET);

    let receiver_mark = receiver.mark();
    receiver.send_line("ne2k mac");
    receiver.expect_output_since(receiver_mark, "[NE2KD] ISA NIC at 0x0300 irq 9", STACK_BUDGET);
    receiver.expect_output_since(receiver_mark, "[NE2KD] irq 9 events bound to IRQ9", STACK_BUDGET);
    receiver.expect_output_since(receiver_mark, "ne2k mac ", FRAME_BUDGET);

    let mut sender = QemuSession::spawn_ne2k_isa(MULTICAST);
    sender.expect_output("rose>", BOOT_BUDGET);

    let sender_mark = sender.mark();
    sender.send_line(&format!("ne2k send {}", sample_frame()));
    sender.expect_output_since(sender_mark, "ne2k sent 60 bytes", FRAME_BUDGET);

    let poll_mark = receiver.mark();
    receiver.send_line("ne2k poll");
    receiver.expect_output_since(
        poll_mark,
        "ne2k frame 60 bytes: de ad be ef ca fe 02 00 00 00 00 01 08 00 52 52",
        FRAME_BUDGET,
    );
}
