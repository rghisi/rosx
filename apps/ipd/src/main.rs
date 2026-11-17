#![no_std]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use ip_protocol::wire::{self as ip_wire, IpError, Reply as IpReply, Request as IpRequest};
use ipd::device::FrameDevice;
use ipd::stack::{Config, Event, Stack};
use net_protocol::client::Ne2k;
use net_protocol::ethernet::{MAC_BYTES, MIN_FRAME_BYTES};
use net_protocol::wire::{Reply as NetReply, Request as NetRequest, MAX_ETHERNET_BYTES};
use usrlib::error::IpcSendError;
use usrlib::handle::{
    UserBindingHandle, UserConnectionHandle, UserDataHandle, UserFutureHandle,
};
use usrlib::syscall::Syscall;
use usrlib::println;

const SERVICE_REQUEST_TAG: u32 = 1;
const NE2K_MESSAGE_TAG: u32 = 2;
const IDLE_DELAY_MS: u64 = 20;
const RETRY_DELAY_MS: u64 = 200;
const LINK_ATTEMPTS: usize = 50;
const PUSH_HELD_FRAMES: usize = 32;
const INCOMING_READ_CHUNK_BYTES: usize = 4096;
const EXIT_NO_DEVICE: u32 = 1;

struct Ne2kDevice {
    connection: UserConnectionHandle,
    mac: [u8; MAC_BYTES],
    pending_sends: VecDeque<UserDataHandle>,
}

impl FrameDevice for Ne2kDevice {
    fn mac(&self) -> [u8; MAC_BYTES] {
        self.mac
    }

    fn transmit(&mut self, frame: &[u8]) {
        if frame.len() > MAX_ETHERNET_BYTES {
            return;
        }
        let mut padded = frame.to_vec();
        if padded.len() < MIN_FRAME_BYTES {
            padded.resize(MIN_FRAME_BYTES, 0);
        }
        let Ok(mut object) = Syscall::ipc_data_write(self.connection, &padded) else {
            return;
        };
        let token = data_token(&object);
        let Ok(message) = NetRequest::Send {
            token,
            length: padded.len() as u16,
        }
        .encode() else {
            let _ = object.release();
            return;
        };
        if Syscall::ipc_send(self.connection, &message).is_err() {
            let _ = object.release();
            return;
        }
        self.pending_sends.push_back(object);
    }
}

impl Ne2kDevice {
    fn collect_send_reply(&mut self) {
        if let Some(mut object) = self.pending_sends.pop_front() {
            let _ = object.release();
        }
    }
}

struct Subscriber {
    connection: UserConnectionHandle,
    icmp_kind: u8,
}

struct Daemon {
    stack: Stack<Ne2kDevice>,
    subscribers: Vec<Subscriber>,
    push_held: Vec<(u32, UserDataHandle)>,
    neigh_held: Vec<(u32, UserDataHandle)>,
}

impl Daemon {
    fn serve_accepted(&mut self, accepted: Option<(UserConnectionHandle, [u8; ip_wire::FRAME_BYTES])>) {
        let Some((connection, request_frame)) = accepted else {
            return;
        };
        let owner = conn_owner(connection);
        self.release_neigh_for(owner);
        let reply = match IpRequest::decode(&request_frame) {
            Ok(request) => self.dispatch(connection, owner, request),
            Err(error) => Some(IpReply::Err { error }),
        };
        if let Some(reply) = reply {
            if let Ok(frame) = reply.encode() {
                let _ = Syscall::ipc_send(connection, &frame);
            }
        }
    }

    fn dispatch(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        request: IpRequest,
    ) -> Option<IpReply> {
        match request {
            IpRequest::Status => Some(IpReply::Status(self.status_snapshot())),
            IpRequest::Config {
                address,
                prefix_len,
                gateway,
            } => {
                self.stack.configure(Config {
                    address,
                    prefix_len,
                    gateway,
                });
                println!(
                    "[IPD] configured {}/{} gw {}",
                    ip_wire::format_address(address),
                    prefix_len,
                    gateway
                        .map(ip_wire::format_address)
                        .unwrap_or_else(|| "none".into())
                );
                Some(IpReply::Ack)
            }
            IpRequest::Neigh => Some(self.neigh_reply(connection, owner)),
            IpRequest::Subscribe { icmp_kind } => {
                match self
                    .subscribers
                    .iter_mut()
                    .find(|subscriber| subscriber.connection == connection)
                {
                    Some(subscriber) => subscriber.icmp_kind = icmp_kind,
                    None => self.subscribers.push(Subscriber {
                        connection,
                        icmp_kind,
                    }),
                }
                Some(IpReply::Ack)
            }
            IpRequest::SendIp {
                destination,
                protocol,
                token,
                length,
            } => {
                match drain_incoming(token).filter(|payload| payload.len() == length as usize) {
                    Some(payload) => {
                        let now_ms = Syscall::now();
                        if let Err(error) =
                            self.stack.send_ip(destination, protocol, &payload, now_ms)
                        {
                            println!(
                                "[IPD] send to {} refused: {:?}",
                                ip_wire::format_address(destination),
                                error
                            );
                        }
                    }
                    None => println!("[IPD] send payload unread"),
                }
                None
            }
        }
    }

    fn status_snapshot(&mut self) -> ip_wire::Status {
        let counters = self.stack.counters();
        let configured = self.stack.config();
        ip_wire::Status {
            link: true,
            configured: configured.is_some(),
            address: configured.map(|config| config.address).unwrap_or([0; 4]),
            prefix_len: configured.map(|config| config.prefix_len).unwrap_or(0),
            gateway: configured.and_then(|config| config.gateway),
            mac: self.stack.mac(),
            counters: ip_wire::StatusCounters {
                rx_frames: counters.rx_frames,
                tx_frames: counters.tx_frames,
                rx_dropped: counters.rx_dropped,
                tx_dropped: counters.tx_dropped,
                arp_answers: counters.arp_answers,
                echo_answers: counters.echo_answers,
            },
        }
    }

    fn neigh_reply(&mut self, connection: UserConnectionHandle, owner: u32) -> IpReply {
        let now_ms = Syscall::now();
        let neighbours = self.stack.neighbours(now_ms);
        let blob = ip_wire::encode_neighbours_table(&neighbours);
        match Syscall::ipc_data_write(connection, &blob) {
            Ok(object) => {
                let token = data_token(&object);
                self.neigh_held.push((owner, object));
                IpReply::Neigh {
                    token,
                    entries: neighbours.len() as u16,
                }
            }
            Err(_) => IpReply::Err {
                error: IpError::Congested,
            },
        }
    }

    fn release_neigh_for(&mut self, owner: u32) {
        self.neigh_held.retain_mut(|(held_owner, object)| {
            if *held_owner == owner {
                let _ = object.release();
                false
            } else {
                true
            }
        });
    }

    fn handle_frame(&mut self, frame: &[u8]) {
        let now_ms = Syscall::now();
        let events = self.stack.on_frame(frame, now_ms);
        self.fan_out(events);
    }

    fn tick(&mut self) {
        let now_ms = Syscall::now();
        let events = self.stack.tick(now_ms);
        self.fan_out(events);
    }

    fn fan_out(&mut self, events: Vec<Event>) {
        for event in events {
            match event {
                Event::Datagram {
                    source,
                    protocol,
                    payload,
                } => self.push_datagram(source, protocol, &payload),
                Event::Unreachable { destination } => {
                    self.push_unreachable(destination, ip_wire::UnreachableReason::NoRoute)
                }
            }
        }
    }

    fn push_datagram(&mut self, source: [u8; 4], protocol: u8, payload: &[u8]) {
        if payload.is_empty()
            || payload.len() > ip_wire::MAX_IP_PAYLOAD_BYTES
            || self.subscribers.is_empty()
        {
            return;
        }
        let icmp_kind = payload[0];
        let subscribers = core::mem::take(&mut self.subscribers);
        let mut keep: Vec<Subscriber> = Vec::new();
        for subscriber in subscribers {
            if subscriber.icmp_kind != ip_wire::KIND_ALL
                && (protocol != 1 || subscriber.icmp_kind != icmp_kind)
            {
                keep.push(subscriber);
                continue;
            }
            let owner = conn_owner(subscriber.connection);
            let Ok(mut object) = Syscall::ipc_data_write(subscriber.connection, payload) else {
                keep.push(subscriber);
                continue;
            };
            let notice = IpReply::PushDatagram {
                source,
                protocol,
                token: data_token(&object),
                length: payload.len() as u16,
            };
            let keep_subscriber = match notice.encode() {
                Err(_) => {
                    let _ = object.release();
                    true
                }
                Ok(frame) => match Syscall::ipc_send(subscriber.connection, &frame) {
                    Ok(()) => {
                        self.hold_push(owner, object);
                        true
                    }
                    Err(IpcSendError::ConnectionNotFound) => {
                        let _ = object.release();
                        false
                    }
                    Err(_) => {
                        let _ = object.release();
                        true
                    }
                },
            };
            if keep_subscriber {
                keep.push(subscriber);
            }
        }
        self.subscribers = keep;
    }

    fn push_unreachable(&mut self, destination: [u8; 4], reason: ip_wire::UnreachableReason) {
        let Ok(frame) = IpReply::PushUnreachable { destination, reason }.encode() else {
            return;
        };
        let subscribers = core::mem::take(&mut self.subscribers);
        let mut keep: Vec<Subscriber> = Vec::new();
        for subscriber in subscribers {
            match Syscall::ipc_send(subscriber.connection, &frame) {
                Err(IpcSendError::ConnectionNotFound) => {}
                _ => keep.push(subscriber),
            }
        }
        self.subscribers = keep;
    }

    fn hold_push(&mut self, owner: u32, object: UserDataHandle) {
        if self
            .push_held
            .iter()
            .filter(|(held_owner, _)| *held_owner == owner)
            .count()
            == PUSH_HELD_FRAMES
        {
            if let Some(oldest) = self
                .push_held
                .iter()
                .position(|(held_owner, _)| *held_owner == owner)
            {
                let (_, mut stale) = self.push_held.remove(oldest);
                let _ = stale.release();
            }
        }
        self.push_held.push((owner, object));
    }

    fn collect_send_reply(&mut self) {
        self.stack.device_mut().collect_send_reply();
    }
}

fn conn_owner(connection: UserConnectionHandle) -> u32 {
    connection.value() as u32
}

fn data_token(handle: &UserDataHandle) -> u32 {
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}

fn drain_incoming(token: u32) -> Option<Vec<u8>> {
    let mut payload = Vec::new();
    loop {
        let mut chunk = [0u8; INCOMING_READ_CHUNK_BYTES];
        let copied = Syscall::ipc_data_read(token as usize, &mut chunk).ok()?;
        if copied == 0 {
            return Some(payload);
        }
        payload.extend_from_slice(&chunk[..copied]);
    }
}

fn mac_text(mac: [u8; MAC_BYTES]) -> String {
    let mut rendered = String::new();
    for (index, byte) in mac.iter().enumerate() {
        if index > 0 {
            rendered.push(':');
        }
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

fn request_mac(connection: UserConnectionHandle) -> Option<[u8; MAC_BYTES]> {
    let message = NetRequest::Mac.encode().ok()?;
    Syscall::ipc_send(connection, &message).ok()?;
    let future = Syscall::ipc_receive_message(connection)?;
    let received = Syscall::wait_for_message(future).ok()?;
    match NetReply::decode(received.data()).ok()? {
        NetReply::Mac(mac) => Some(mac),
        _ => None,
    }
}

fn main() {
    println!("[IPD] ip stack daemon");
    let service = bind_with_retry();
    let connection = attach_link();
    let mac = request_mac(connection).unwrap_or([0u8; MAC_BYTES]);
    println!("[IPD] link up on NE2K mac {}", mac_text(mac));
    let mut daemon = Daemon {
        stack: Stack::new(Ne2kDevice {
            connection,
            mac,
            pending_sends: VecDeque::new(),
        }),
        subscribers: Vec::new(),
        push_held: Vec::new(),
        neigh_held: Vec::new(),
    };
    println!("[IPD] bound {}, awaiting configuration", ip_wire::SERVICE_NAME);
    let mut request_future: Option<UserFutureHandle> = None;
    let mut message_future: Option<UserFutureHandle> = None;
    loop {
        if request_future.is_none() {
            request_future = Syscall::ipc_accept_message(service);
        }
        if message_future.is_none() {
            message_future = Syscall::ipc_receive_message(connection);
        }
        let (Some(request), Some(message)) = (request_future, message_future) else {
            Syscall::sleep(RETRY_DELAY_MS);
            continue;
        };
        let Ok(set) = Syscall::waitset_create() else {
            Syscall::sleep(IDLE_DELAY_MS);
            continue;
        };
        if Syscall::waitset_attach(set, request, SERVICE_REQUEST_TAG).is_err() {
            Syscall::waitset_drop(set);
            request_future = None;
            Syscall::sleep(IDLE_DELAY_MS);
            continue;
        }
        if Syscall::waitset_attach(set, message, NE2K_MESSAGE_TAG).is_err() {
            message_future = None;
        }
        match Syscall::waitset_wait(set) {
            Ok((child, NE2K_MESSAGE_TAG)) => {
                message_future = None;
                match Syscall::wait_for_message(child) {
                    Ok(received) if received.connection().is_some() => match NetReply::decode(received.data()) {
                        Ok(NetReply::PushFrame { token, length }) => {
                            if let Some(frame) =
                                drain_incoming(token).filter(|frame| frame.len() == length as usize)
                            {
                                daemon.handle_frame(&frame);
                            }
                        }
                        Ok(NetReply::Ack) | Ok(NetReply::Err { .. }) => daemon.collect_send_reply(),
                        _ => {}
                    },
                    _ => {
                        println!("[IPD] NE2K connection lost");
                        Syscall::sleep(RETRY_DELAY_MS);
                    }
                }
            }
            Ok((child, _)) => {
                request_future = None;
                daemon.serve_accepted(accepted_request(child));
            }
            Err(_) => {
                Syscall::sleep(IDLE_DELAY_MS);
            }
        }
        Syscall::waitset_drop(set);
        daemon.tick();
    }
}

fn accepted_request(child: UserFutureHandle) -> Option<(UserConnectionHandle, [u8; ip_wire::FRAME_BYTES])> {
    let received = Syscall::wait_for_message(child).ok()?;
    let connection = received.connection()?;
    Some((connection, *received.data()))
}

fn bind_with_retry() -> UserBindingHandle {
    loop {
        if let Ok(binding) = Syscall::ipc_bind(ip_wire::SERVICE_NAME) {
            return binding;
        }
        Syscall::sleep(RETRY_DELAY_MS);
    }
}

fn attach_link() -> UserConnectionHandle {
    for _ in 0..LINK_ATTEMPTS {
        if let Ok(nic) = Ne2k::connect() {
            if nic.watch().is_ok() {
                return nic.connection();
            }
        }
        Syscall::sleep(RETRY_DELAY_MS);
    }
    println!("[IPD] no NE2K service, exiting");
    Syscall::exit(EXIT_NO_DEVICE);
}
