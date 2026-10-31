use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const REPLY_BUDGET: Duration = Duration::from_secs(90);

#[test]
fn ata_stress_reads_chunks_and_writebacks_twice() {
    let mut session = QemuSession::spawn_with_scratch_disk();
    session.expect_output("[ATAD] ata0 drive1 ready:", BOOT_BUDGET);
    session.expect_output("rose>", BOOT_BUDGET);

    let mark = session.mark();
    session.send_line("ata stress");
    session.expect_output_since(mark, "stress read ok chunks=4", REPLY_BUDGET);
    session.expect_output_since(mark, "stress writeback ok cycles=2", REPLY_BUDGET);
    session.expect_output_since(mark, "rose>", REPLY_BUDGET);
}
