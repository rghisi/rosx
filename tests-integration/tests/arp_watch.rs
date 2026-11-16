use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(240);
const COMMAND_BUDGET: Duration = Duration::from_secs(90);
const INPUT_ATTEMPTS: usize = 5;
const MULTICAST: &str = "230.0.0.1:19421";

fn who_has() -> Vec<u8> {
    let mut frame = vec![0xffu8; 6];
    frame.extend_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
    frame.extend_from_slice(&[0x08, 0x06]);
    frame.extend_from_slice(&[0x00, 0x01, 0x08, 0x00, 0x06, 0x04, 0x00, 0x01]);
    frame.extend_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
    frame.extend_from_slice(&[192, 168, 7, 5]);
    frame.extend_from_slice(&[0u8; 6]);
    frame.extend_from_slice(&[192, 168, 7, 1]);
    frame.resize(60, 0);
    frame
}

#[test]
fn arp_watch_decodes_injected_frame() {
    let mut session = QemuSession::spawn_ne2k_isa(MULTICAST);
    session.expect_output("rose>", BOOT_BUDGET);

    let socket = UdpSocket::bind("0.0.0.0:0").expect("udp socket");
    let frame = who_has();
    let injector = thread::spawn(move || {
        let target: std::net::SocketAddr = MULTICAST.parse().expect("multicast address");
        let first = Instant::now() + Duration::from_secs(5);
        while Instant::now() < first {
            thread::sleep(Duration::from_millis(500));
        }
        for _ in 0..10 {
            let _ = socket.send_to(&frame, target);
            thread::sleep(Duration::from_secs(1));
        }
    });

    let transcript = session.run_command(
        "arp watch 8",
        |delta| delta.contains("arp watch done"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    eprintln!("WATCH TRANSCRIPT START\n{transcript}\nWATCH TRANSCRIPT END");
    assert!(
        transcript.contains("arp request: who-has 192.168.7.1 tell 192.168.7.5"),
        "injected who-has frame was not decoded by arp watch"
    );
    let _ = injector.join();
}
