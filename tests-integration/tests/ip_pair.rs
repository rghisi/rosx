use std::thread;
use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(240);
const COMMAND_BUDGET: Duration = Duration::from_secs(120);
const INPUT_ATTEMPTS: usize = 5;
const MULTICAST: &str = "230.0.0.1:19431";

#[test]
fn ip_pair_pings_across_the_multicast_medium() {
    let mut responder = QemuSession::spawn_ne2k_isa(MULTICAST);
    let mut requester = QemuSession::spawn_ne2k_isa(MULTICAST);
    responder.expect_output("rose>", BOOT_BUDGET);
    requester.expect_output("rose>", BOOT_BUDGET);

    responder.run_command(
        "ip config 10.9.9.1/24",
        |delta| delta.contains("ip configured 10.9.9.1/24 gw none"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    requester.run_command(
        "ip config 10.9.9.2/24",
        |delta| delta.contains("ip configured 10.9.9.2/24 gw none"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );

    let watch_mark = responder.mark();
    responder.send_line("arp watch 25");
    thread::sleep(Duration::from_secs(3));

    let ping = requester.run_command(
        "ip ping 10.9.9.1 2",
        |delta| delta.contains("% loss"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    eprintln!("PAIR PING START\n{ping}\nPAIR PING END");
    assert!(
        ping.contains("2 received"),
        "the peer ipd never answered the echo requests\n{ping}"
    );

    responder.expect_output_since(
        watch_mark,
        "arp request: who-has 10.9.9.1 tell 10.9.9.2",
        Duration::from_secs(60),
    );
    responder.expect_output_since(watch_mark, "arp watch done", Duration::from_secs(60));

    let requester_table = requester.run_command(
        "ip neigh",
        |delta| delta.contains("10.9.9.1") || delta.contains("ip neigh empty"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    assert!(
        requester_table.contains("10.9.9.1"),
        "requester never learned the responder\n{requester_table}"
    );
    let responder_table = responder.run_command(
        "ip neigh",
        |delta| delta.contains("10.9.9.2") || delta.contains("ip neigh empty"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    assert!(
        responder_table.contains("10.9.9.2"),
        "responder never learned the requester\n{responder_table}"
    );
}
