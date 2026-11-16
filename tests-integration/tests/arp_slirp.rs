use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(240);
const COMMAND_BUDGET: Duration = Duration::from_secs(90);
const INPUT_ATTEMPTS: usize = 5;

#[test]
fn arp_who_has_answers_through_slirp() {
    let mut session = QemuSession::spawn_ne2k_pci();
    session.expect_output("rose>", BOOT_BUDGET);

    let transcript = session.run_command(
        "arp who-has 10.0.2.2 from 10.0.2.15",
        |delta| delta.contains("arp 10.0.2.2 is at") || delta.contains("arp no reply"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    eprintln!("SLIRP WHO-HAS START\n{transcript}\nSLIRP WHO-HAS END");
    assert!(transcript.contains("arp 10.0.2.2 is at"), "slirp did not answer who-has");
}
