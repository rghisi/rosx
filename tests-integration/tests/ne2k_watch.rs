use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(240);
const COMMAND_BUDGET: Duration = Duration::from_secs(90);
const INPUT_ATTEMPTS: usize = 5;
const MULTICAST: &str = "230.0.0.1:19427";

fn injected_frame() -> Vec<u8> {
    let mut frame = vec![0xffu8; 6];
    frame.extend_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
    frame.extend_from_slice(&[0x08, 0x06]);
    frame.extend_from_slice(&[0x00, 0x01, 0x08, 0x00, 0x06, 0x04, 0x00, 0x01]);
    frame.extend_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
    frame.extend_from_slice(&[192, 168, 9, 5]);
    frame.extend_from_slice(&[0u8; 6]);
    frame.extend_from_slice(&[192, 168, 9, 1]);
    frame.resize(60, 0);
    frame
}

#[test]
fn ne2k_watch_receives_pushed_frame() {
    let mut session = QemuSession::spawn_ne2k_isa(MULTICAST);
    session.expect_output("rose>", BOOT_BUDGET);

    let socket = UdpSocket::bind("0.0.0.0:0").expect("udp socket");
    let frame = injected_frame();
    let done = Arc::new(AtomicBool::new(false));
    let stopped = done.clone();
    let injector = thread::spawn(move || {
        let target: std::net::SocketAddr = MULTICAST.parse().expect("multicast address");
        while !stopped.load(Ordering::Relaxed) {
            let _ = socket.send_to(&frame, target);
            thread::sleep(Duration::from_millis(250));
        }
    });

    let transcript = session.run_command(
        "ne2k watch 1",
        |delta| delta.contains("ne2k frame"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    done.store(true, Ordering::Relaxed);
    let _ = injector.join();
    assert!(
        transcript.contains("ne2k frame 60 bytes"),
        "pushed watch frame was not delivered to ne2k watch\n{transcript}"
    );
}
