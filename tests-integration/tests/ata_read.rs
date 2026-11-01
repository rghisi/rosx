use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const REPLY_BUDGET: Duration = Duration::from_secs(30);

#[test]
fn ata_client_reads_the_mbr_through_atad() {
    let mut session = QemuSession::spawn();
    session.expect_output("rose>", BOOT_BUDGET);

    let mark = session.mark();
    session.send_line("ata");
    session.expect_output_since(mark, "model=", REPLY_BUDGET);
    session.expect_output_since(mark, "mbr signature 55AA ok", REPLY_BUDGET);
    session.expect_output_since(mark, "mbr part3", REPLY_BUDGET);
    session.expect_output_since(mark, "rose>", REPLY_BUDGET);
}
