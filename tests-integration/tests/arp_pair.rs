use std::thread;
use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(240);
const COMMAND_BUDGET: Duration = Duration::from_secs(90);
const INPUT_ATTEMPTS: usize = 5;
const MULTICAST: &str = "230.0.0.1:19424";

#[test]
fn arp_pair_request_reply_and_announce() {
    let mut responder = QemuSession::spawn_ne2k_isa(MULTICAST);
    let mut requester = QemuSession::spawn_ne2k_isa(MULTICAST);
    responder.expect_output("rose>", BOOT_BUDGET);
    requester.expect_output("rose>", BOOT_BUDGET);

    let responder_mark = responder.mark();
    responder.send_line("arp respond 192.168.99.7 25");
    thread::sleep(Duration::from_secs(6));

    let asked = requester.run_command(
        "arp who-has 192.168.99.7 from 192.168.99.5",
        |delta| delta.contains("arp 192.168.99.7 is at"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    eprintln!("REQUESTER WHO-HAS START\n{asked}\nREQUESTER WHO-HAS END");
    assert!(asked.contains("arp 192.168.99.7 is at"), "requester got no answer");

    responder.expect_output_since(responder_mark, "arp respond done", Duration::from_secs(60));
    let window = responder.content_since(responder_mark);
    eprintln!("RESPONDER WINDOW START\n{window}\nRESPONDER WINDOW END");
    assert!(
        window.contains("arp replied to 192.168.99.5"),
        "responder never answered"
    );

    let watch_mark = responder.mark();
    responder.send_line("arp watch 12");
    thread::sleep(Duration::from_secs(4));
    let announced = requester.run_command(
        "arp announce 192.168.99.5",
        |delta| delta.contains("arp announced 192.168.99.5 is at"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    eprintln!("ANNOUNCE START\n{announced}\nANNOUNCE END");
    assert!(announced.contains("arp announced 192.168.99.5 is at"), "announce report missing");
    responder.expect_output_since(watch_mark, "arp reply: 192.168.99.5", Duration::from_secs(25));
}
