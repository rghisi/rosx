#![no_std]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use ne2kd::dp8390::{
    Ne2k, NicBus, NicCounters, NicFault, ETHERNET_MAX_BYTES, ISA_DEFAULT_BASE, ISA_DEFAULT_IRQ,
    MAC_BYTES,
};
use net_protocol::wire::{self, NetError, Reply, Request, Stats};
use pci_protocol::client::Pci;
use usrlib::error::{IpcReceiveError, PortError};
use usrlib::handle::{UserBindingHandle, UserConnectionHandle, UserDataHandle};
use usrlib::ipc::ReceivedMessage;
use usrlib::println;
use usrlib::syscall::Syscall;

const RTL8029_VENDOR_ID: u16 = 0x10EC;
const RTL8029_DEVICE_ID: u16 = 0x8029;
const CARD_PORT_SPAN: u16 = 0x1F;
const ISA_BASES: [u16; 2] = [ISA_DEFAULT_BASE, 0x280];
const RX_QUEUE_FRAMES: usize = 32;
const REBIND_DELAY_MS: u64 = 100;
const IDLE_DELAY_MS: u64 = 10;
const INCOMING_READ_CHUNK_BYTES: usize = 4096;
const IRQ_EVENT_TAG: u32 = 1;
const SERVICE_REQUEST_TAG: u32 = 2;
const EXIT_NO_DEVICE: u32 = 1;

struct SyscallNicBus;

impl NicBus for SyscallNicBus {
    fn read_byte(&mut self, port: u16) -> Result<u8, PortError> {
        Syscall::port_in(port, 1).map(|value| value as u8)
    }

    fn write_byte(&mut self, port: u16, value: u8) -> Result<(), PortError> {
        Syscall::port_out(port, value as usize, 1)
    }

    fn read_words(&mut self, port: u16, destination: &mut [u16]) -> Result<(), PortError> {
        Syscall::port_in_words(port, destination)
    }

    fn write_words(&mut self, port: u16, source: &[u16]) -> Result<(), PortError> {
        Syscall::port_out_words(port, source)
    }

    fn sleep_ms(&mut self, millis: u64) {
        Syscall::sleep(millis);
    }
}

struct AttachedNic {
    nic: Ne2k<SyscallNicBus>,
    irq: u32,
    kind: &'static str,
}

fn attach_pci_rtl8029() -> Option<AttachedNic> {
    println!("[NE2KD] searching for RTL8029AS through pcid");
    let pci = Pci::connect().ok()?;
    let found = pci.find(RTL8029_VENDOR_ID, RTL8029_DEVICE_ID).ok()??;
    let enabled = pci.enable(&found).ok()?;
    let base = (enabled.bar0 & 0xFFFC) as u16;
    if base == 0 {
        return None;
    }
    Syscall::port_acquire(base, base + CARD_PORT_SPAN).ok()?;
    let mut nic = Ne2k::new(base, SyscallNicBus);
    match nic.initialize() {
        Ok(_) => Some(AttachedNic {
            nic,
            irq: enabled.irq as u32,
            kind: "PCI",
        }),
        Err(fault) => {
            println!("[NE2KD] PCI card at {base:#06x} refused init: {fault}");
            let _ = Syscall::port_release(base, base + CARD_PORT_SPAN);
            None
        }
    }
}

fn attach_isa_card() -> Option<AttachedNic> {
    for base in ISA_BASES {
        if Syscall::port_acquire(base, base + CARD_PORT_SPAN).is_err() {
            continue;
        }
        let mut nic = Ne2k::new(base, SyscallNicBus);
        if nic.initialize().is_ok() {
            return Some(AttachedNic {
                nic,
                irq: ISA_DEFAULT_IRQ,
                kind: "ISA",
            });
        }
        let _ = Syscall::port_release(base, base + CARD_PORT_SPAN);
    }
    None
}

fn attach_nic() -> Option<AttachedNic> {
    attach_pci_rtl8029().or_else(attach_isa_card)
}

fn bind_event_source(irq: u32) -> Option<UserBindingHandle> {
    if !system::irq::is_subscribable(irq) {
        println!("[NE2KD] irq {irq} is not subscribable, poll-only mode");
        return None;
    }
    let name = system::irq::SOURCE_NAMES[irq as usize];
    let binding = Syscall::ipc_bind(name).ok()?;
    match Syscall::irq_subscribe(irq) {
        Ok(()) => {
            println!("[NE2KD] irq {irq} events bound to {name}");
            Some(binding)
        }
        Err(error) => {
            println!("[NE2KD] irq {irq} subscribe refused ({error}), poll-only mode");
            None
        }
    }
}

fn conn_token(connection: UserConnectionHandle) -> u32 {
    u32::from_str_radix(&format!("{}", connection), 16).expect("canonical connection token")
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

fn format_mac(mac: [u8; MAC_BYTES]) -> String {
    let mut rendered = String::new();
    for (index, byte) in mac.iter().enumerate() {
        if index > 0 {
            rendered.push(':');
        }
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

fn stats_of(counters: NicCounters) -> Stats {
    Stats {
        tx_frames: counters.transmitted_frames,
        tx_errors: counters.transmit_errors,
        rx_frames: counters.received_frames,
        rx_errors: counters.receive_errors,
        rx_overruns: counters.receive_overruns,
    }
}

fn net_error_of(fault: NicFault) -> NetError {
    match fault {
        NicFault::Absent => NetError::NoDevice,
        NicFault::TransmitBusy => NetError::Busy,
        NicFault::Oversize => NetError::Oversize,
        _ => NetError::Device,
    }
}

struct Ne2kServer {
    nic: Ne2k<SyscallNicBus>,
    irq: u32,
    rx_queue: VecDeque<Vec<u8>>,
    held: Vec<(u32, UserDataHandle)>,
}

impl Ne2kServer {
    fn new(nic: Ne2k<SyscallNicBus>, irq: u32) -> Ne2kServer {
        Ne2kServer {
            nic,
            irq,
            rx_queue: VecDeque::new(),
            held: Vec::new(),
        }
    }

    fn serve(&mut self, service: UserBindingHandle, events: Option<UserBindingHandle>) {
        loop {
            let Some(request_future) = Syscall::ipc_accept_message(service) else {
                Syscall::sleep(REBIND_DELAY_MS);
                continue;
            };
            let Ok(set) = Syscall::waitset_create() else {
                self.serve_accepted(Syscall::wait_for_message(request_future));
                continue;
            };
            if Syscall::waitset_attach(set, request_future, SERVICE_REQUEST_TAG).is_err() {
                Syscall::waitset_drop(set);
                Syscall::sleep(IDLE_DELAY_MS);
                continue;
            }
            let event_future = events
                .and_then(Syscall::ipc_accept_message)
                .filter(|future| Syscall::waitset_attach(set, *future, IRQ_EVENT_TAG).is_ok());
            match Syscall::waitset_wait(set) {
                Ok((child, IRQ_EVENT_TAG)) => {
                    let _ = Syscall::wait_for_message(child);
                    self.on_irq_event();
                }
                Ok((child, _)) => {
                    self.serve_accepted(Syscall::wait_for_message(child));
                }
                Err(_) => {
                    let _ = event_future;
                    Syscall::sleep(IDLE_DELAY_MS);
                }
            }
            Syscall::waitset_drop(set);
        }
    }

    fn serve_accepted(&mut self, received: Result<ReceivedMessage, IpcReceiveError>) {
        let Ok(received) = received else {
            return;
        };
        let Some(connection) = received.connection() else {
            return;
        };
        let owner = conn_token(connection);
        self.release_held_for(owner);
        let reply = match Request::decode(received.data()) {
            Ok(request) => self.dispatch(connection, owner, request),
            Err(error) => Reply::Err { error },
        };
        if let Ok(frame) = reply.encode() {
            let _ = Syscall::ipc_send(connection, &frame);
        }
    }

    fn dispatch(
        &mut self,
        connection: UserConnectionHandle,
        owner: u32,
        request: Request,
    ) -> Reply {
        match request {
            Request::Mac => Reply::Mac(self.nic.mac()),
            Request::Send { token, length } => self.send_reply(token, length),
            Request::Poll => self.poll_reply(connection, owner),
            Request::Stats => Reply::Stats(stats_of(self.nic.counters())),
        }
    }

    fn send_reply(&mut self, token: u32, length: u16) -> Reply {
        let Some(payload) = drain_incoming(token) else {
            return Reply::Err {
                error: NetError::Protocol,
            };
        };
        if payload.len() != length as usize || length as usize > ETHERNET_MAX_BYTES {
            return Reply::Err {
                error: NetError::Protocol,
            };
        }
        match self.nic.transmit(&payload) {
            Ok(()) => Reply::Ack,
            Err(fault) => Reply::Err {
                error: net_error_of(fault),
            },
        }
    }

    fn poll_reply(&mut self, connection: UserConnectionHandle, owner: u32) -> Reply {
        if self.rx_queue.is_empty() {
            self.drain_ring();
        }
        let Some(frame) = self.rx_queue.pop_front() else {
            return Reply::NoFrame;
        };
        match Syscall::ipc_data_write(connection, &frame) {
            Ok(object) => {
                let token = data_token(&object);
                self.held.push((owner, object));
                Reply::Data {
                    token,
                    length: frame.len() as u16,
                }
            }
            Err(_) => Reply::Err {
                error: NetError::Device,
            },
        }
    }

    fn on_irq_event(&mut self) {
        if let Ok(status) = self.nic.clear_interrupt_flags() {
            if status & ne2kd::dp8390::ISR_OVERRUN != 0 {
                let _ = self.nic.recover_overrun();
            }
        }
        self.drain_ring();
        let _ = Syscall::irq_clear(self.irq);
    }

    fn drain_ring(&mut self) {
        while let Ok(Some(frame)) = self.nic.receive() {
            if self.rx_queue.len() == RX_QUEUE_FRAMES {
                self.rx_queue.pop_front();
            }
            self.rx_queue.push_back(frame);
        }
    }

    fn release_held_for(&mut self, owner: u32) {
        self.held.retain_mut(|(held_owner, object)| {
            if *held_owner == owner {
                let _ = object.release();
                false
            } else {
                true
            }
        });
    }
}

fn main() {
    println!("[NE2KD] NE2000 network driver");
    let Some(attached) = attach_nic() else {
        println!("[NE2KD] no NE2000-compatible NIC found");
        Syscall::exit(EXIT_NO_DEVICE);
    };
    let AttachedNic { nic, irq, kind } = attached;
    println!(
        "[NE2KD] {kind} NIC at {:#06x} irq {} mac {}",
        nic.base(),
        irq,
        format_mac(nic.mac())
    );
    let events = bind_event_source(irq);
    let mut server = Ne2kServer::new(nic, irq);
    loop {
        match Syscall::ipc_bind(wire::SERVICE_NAME) {
            Ok(binding) => server.serve(binding, events),
            Err(_) => Syscall::sleep(REBIND_DELAY_MS),
        }
    }
}
