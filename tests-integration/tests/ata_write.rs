use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const REPLY_BUDGET: Duration = Duration::from_secs(60);

#[test]
fn ata_write_roundtrips_on_the_scratch_slave() {
    if tests_integration::arch::not_ported_to_x86_32() {
        return;
    }
    let mut session = QemuSession::spawn_with_scratch_disk();
    session.expect_output("[ATAD] ata0 drive0 ready:", BOOT_BUDGET);
    session.expect_output("[ATAD] ata0 drive1 ready:", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    let mark = session.mark();
    session.send_line("ata write 1 0");
    session.expect_output_since(mark, "writeback ok drive=1 lba=0", REPLY_BUDGET);
    session.expect_output_since(mark, "rose>", REPLY_BUDGET);
}
