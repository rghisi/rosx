use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(240);
const COMMAND_BUDGET: Duration = Duration::from_secs(120);
const INPUT_ATTEMPTS: usize = 5;

#[test]
fn ip_ping_crosses_slirp() {
    let mut session = QemuSession::spawn_ne2k_pci();
    session.expect_output("rose>", BOOT_BUDGET);

    let status = session.run_command(
        "ip status",
        |delta| delta.contains("ip counters") || delta.contains("ip no service"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    eprintln!("SLIRP STATUS START\n{status}\nSLIRP STATUS END");
    assert!(
        status.contains("ip link=up"),
        "ipd never brought the link up\n{status}"
    );

    session.run_command(
        "ip config 10.0.2.15/24 gw 10.0.2.2",
        |delta| delta.contains("ip configured 10.0.2.15/24 gw 10.0.2.2"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );

    let ping = session.run_command(
        "ip ping 10.0.2.2 2",
        |delta| delta.contains("% loss"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    eprintln!("SLIRP PING START\n{ping}\nSLIRP PING END");
    assert!(ping.contains("2 received"), "slirp never answered the ip ping\n{ping}");

    let neighbours = session.run_command(
        "ip neigh",
        |delta| delta.contains("10.0.2.2") || delta.contains("ip neigh empty"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    assert!(
        neighbours.contains("10.0.2.2"),
        "gateway never entered the neighbour table\n{neighbours}"
    );
}
