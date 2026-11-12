use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(120);
const REPLY_BUDGET: Duration = Duration::from_secs(60);
const MULTICAST: &str = "230.0.0.1:19418";

#[test]
fn ne2k_cli_drives_the_stack_on_one_guest() {
    let mut session = QemuSession::spawn_ne2k_isa(MULTICAST);
    session.expect_output("rose>", BOOT_BUDGET);

    let mark = session.mark();
    session.send_line("ne2k mac");
    session.expect_output_since(mark, "[NE2KD] ISA NIC at 0x0300 irq 9", REPLY_BUDGET);
    session.expect_output_since(mark, "ne2k mac ", REPLY_BUDGET);

    let stats_mark = session.mark();
    session.send_line("ne2k stats");
    session.expect_output_since(stats_mark, "ne2k tx=", REPLY_BUDGET);

    let send_mark = session.mark();
    session.send_line("ne2k send deadbeefcafe0200000000010800525252525252525252525252525252525252525252525252525252525252525252525252525252525252525252");
    session.expect_output_since(send_mark, "ne2k sent 60 bytes", REPLY_BUDGET);

    let reread_mark = session.mark();
    session.send_line("ne2k stats");
    session.expect_output_since(reread_mark, "ne2k tx=1 tx-errors=0", REPLY_BUDGET);
}
