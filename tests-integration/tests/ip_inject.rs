use std::net::{SocketAddr, UdpSocket};
use std::os::unix::io::AsRawFd;
use std::thread;
use std::time::{Duration, Instant};
use tests_integration::qemu::QemuSession;
use tests_integration::shared::start_ipd;

const BOOT_BUDGET: Duration = Duration::from_secs(240);
const COMMAND_BUDGET: Duration = Duration::from_secs(120);
const INPUT_ATTEMPTS: usize = 5;
const MULTICAST: &str = "230.0.0.1:19434";
const INJECT_BUDGET: Duration = Duration::from_secs(60);
const HOST_MAC: [u8; 6] = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
const OWN_IP: [u8; 4] = [10, 9, 9, 1];
const HOST_IP: [u8; 4] = [10, 9, 9, 5];
const ECHO_ID: u16 = 0x4944;
const ECHO_SEQ: u16 = 7;
const ECHO_BODY: &[u8] = b"ipd-inject";

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut index = 0;
    while index + 1 < bytes.len() {
        sum += u32::from(u16::from_be_bytes([bytes[index], bytes[index + 1]]));
        index += 2;
    }
    if index < bytes.len() {
        sum += u32::from(bytes[index]) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn who_has() -> Vec<u8> {
    let mut frame = vec![0xffu8; 6];
    frame.extend_from_slice(&HOST_MAC);
    frame.extend_from_slice(&[0x08, 0x06]);
    frame.extend_from_slice(&[0x00, 0x01, 0x08, 0x00, 0x06, 0x04, 0x00, 0x01]);
    frame.extend_from_slice(&HOST_MAC);
    frame.extend_from_slice(&HOST_IP);
    frame.extend_from_slice(&[0u8; 6]);
    frame.extend_from_slice(&OWN_IP);
    frame.resize(60, 0);
    frame
}

fn echo_request() -> Vec<u8> {
    let mut message = vec![8u8, 0, 0, 0];
    message.extend_from_slice(&ECHO_ID.to_be_bytes());
    message.extend_from_slice(&ECHO_SEQ.to_be_bytes());
    message.extend_from_slice(ECHO_BODY);
    let message_sum = checksum(&message).to_be_bytes();
    message[2..4].copy_from_slice(&message_sum);
    let mut datagram = vec![0x45u8, 0x00];
    datagram.extend_from_slice(&((20 + message.len()) as u16).to_be_bytes());
    datagram.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x40, 0x01, 0x00, 0x00]);
    datagram.extend_from_slice(&HOST_IP);
    datagram.extend_from_slice(&OWN_IP);
    let datagram_sum = checksum(&datagram).to_be_bytes();
    datagram[10..12].copy_from_slice(&datagram_sum);
    let mut frame = vec![0xffu8; 6];
    frame.extend_from_slice(&HOST_MAC);
    frame.extend_from_slice(&[0x08, 0x00]);
    frame.extend_from_slice(&datagram);
    frame.extend_from_slice(&message);
    frame.resize(60, 0);
    frame
}

fn witness_socket() -> UdpSocket {
    let (_group, port) = MULTICAST.split_once(':').expect("multicast address");
    let port: u16 = port.parse().expect("port");
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    assert!(fd >= 0, "witness socket failed");
    let on = 1 as libc::c_int;
    let reuse = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &on as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    assert_eq!(reuse, 0, "reuse address failed");
    let address = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: port.to_be(),
        sin_addr: libc::in_addr { s_addr: 0 },
        sin_zero: [0; 8],
    };
    let bound = unsafe {
        libc::bind(
            fd,
            &address as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    assert_eq!(bound, 0, "witness bind failed");
    let socket: UdpSocket = unsafe { std::os::fd::FromRawFd::from_raw_fd(fd) };
    let octets: Vec<u8> = MULTICAST
        .split(':')
        .next()
        .expect("group")
        .split('.')
        .map(|part| part.parse::<u8>().expect("octet"))
        .collect();
    let membership = libc::ip_mreq {
        imr_multiaddr: libc::in_addr {
            s_addr: u32::from_ne_bytes([octets[0], octets[1], octets[2], octets[3]]),
        },
        imr_interface: libc::in_addr { s_addr: 0 },
    };
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_ADD_MEMBERSHIP,
            &membership as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::ip_mreq>() as libc::socklen_t,
        )
    };
    assert_eq!(result, 0, "multicast group membership failed");
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("read timeout");
    socket
}

fn is_arp_reply(frame: &[u8]) -> bool {
    frame.len() >= 42
        && frame[12..14] == [0x08, 0x06]
        && frame[20..22] == [0x00, 0x02]
        && frame[28..32] == OWN_IP
        && frame[38..42] == HOST_IP
        && frame[22..28] != [0u8; 6]
}

fn is_echo_reply(frame: &[u8]) -> bool {
    if frame.len() < 14 || frame[12..14] != [0x08, 0x00] {
        return false;
    }
    let header_len = usize::from(frame[14] & 0x0f) * 4;
    if header_len < 20 || frame[14 + 9] != 1 || frame[26..30] != OWN_IP || frame[30..34] != HOST_IP {
        return false;
    }
    let total_len = usize::from(u16::from_be_bytes([frame[16], frame[17]]));
    if total_len < header_len || 14 + total_len > frame.len() {
        return false;
    }
    let message = &frame[14 + header_len..14 + total_len];
    message.len() >= 8
        && message[0] == 0
        && message[1] == 0
        && u16::from_be_bytes([message[4], message[5]]) == ECHO_ID
        && u16::from_be_bytes([message[6], message[7]]) == ECHO_SEQ
        && &message[8..] == ECHO_BODY
}

fn witness_holds(socket: &UdpSocket, accepted: fn(&[u8]) -> bool) -> bool {
    let mut buffer = [0u8; 2048];
    match socket.recv(&mut buffer) {
        Ok(0) => false,
        Ok(read) => accepted(&buffer[..read]),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
        Err(_) => false,
    }
}

fn inject_until_witnessed(socket: &UdpSocket, frame: &[u8], accepted: fn(&[u8]) -> bool) -> bool {
    let deadline = Instant::now() + INJECT_BUDGET;
    while Instant::now() < deadline {
        let target: SocketAddr = MULTICAST.parse().expect("multicast address");
        let _ = socket.send_to(frame, target);
        let drain_deadline = Instant::now() + Duration::from_millis(600);
        while Instant::now() < drain_deadline {
            if witness_holds(socket, accepted) {
                return true;
            }
        }
    }
    false
}

#[test]
fn injected_frames_are_answered_by_ipd() {
    let mut session = QemuSession::spawn_ne2k_isa(MULTICAST);
    session.expect_output("rose>", BOOT_BUDGET);
    start_ipd(&mut session);

    session.run_command(
        "ip config 10.9.9.1/24",
        |delta| delta.contains("ip configured 10.9.9.1/24 gw none"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );

    let socket = witness_socket();
    let arp_answered = inject_until_witnessed(&socket, &who_has(), is_arp_reply);
    assert!(arp_answered, "ipd never answered the injected who-has on the medium");

    thread::sleep(Duration::from_millis(250));
    let echo_answered = inject_until_witnessed(&socket, &echo_request(), is_echo_reply);
    let counters = session.run_command(
        "ip status",
        |delta| delta.contains("ip counters"),
        INPUT_ATTEMPTS,
        COMMAND_BUDGET,
    );
    eprintln!("INJECT STATUS START\n{counters}\nINJECT STATUS END");
    assert!(
        echo_answered,
        "ipd never echoed the injected request on the medium\n{counters}"
    );
}
