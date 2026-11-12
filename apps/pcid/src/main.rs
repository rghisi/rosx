#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use pci_protocol::wire::{self, Reply, Request, Slot};
use pcid::enumeration::{
    self, scan_bus, ConfigBus, CONFIG_ADDRESS_PORT, CONFIG_DATA_PORT, CONFIG_PORTS_END,
};
use usrlib::handle::{UserBindingHandle, UserConnectionHandle, UserDataHandle};
use usrlib::println;
use usrlib::syscall::Syscall;

const REBIND_DELAY_MS: u64 = 100;
const EXIT_ACQUIRE_REFUSED: u32 = 1;
const SCANNED_BUS: u8 = 0;

struct SyscallConfigBus;

impl ConfigBus for SyscallConfigBus {
    fn read_dword(&mut self, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        let address = enumeration::config_address(bus, device, function, offset);
        if Syscall::port_out(CONFIG_ADDRESS_PORT, address as usize, 4).is_err() {
            return 0xFFFF_FFFF;
        }
        Syscall::port_in(CONFIG_DATA_PORT, 4).unwrap_or(0xFFFF_FFFF)
    }

    fn write_dword(&mut self, bus: u8, device: u8, function: u8, offset: u8, value: u32) {
        let address = enumeration::config_address(bus, device, function, offset);
        let _ = Syscall::port_out(CONFIG_ADDRESS_PORT, address as usize, 4);
        let _ = Syscall::port_out(CONFIG_DATA_PORT, value as usize, 4);
    }
}

fn conn_token(connection: UserConnectionHandle) -> u32 {
    use alloc::format;
    u32::from_str_radix(&format!("{}", connection), 16).expect("canonical connection token")
}

fn data_token(handle: &UserDataHandle) -> u32 {
    use alloc::format;
    u32::from_str_radix(&format!("{}", handle), 16).expect("canonical data token")
}

fn main() {
    println!("[PCID] PCI enumeration service");
    if let Err(error) = Syscall::port_acquire(CONFIG_ADDRESS_PORT, CONFIG_PORTS_END) {
        println!("[PCID] configuration ports refused: {error}");
        Syscall::exit(EXIT_ACQUIRE_REFUSED);
    }
    let mut slots = scan_bus(SCANNED_BUS, &mut SyscallConfigBus);
    println!("[PCID] bus {SCANNED_BUS}: {} functions present", slots.len());
    for slot in &slots {
        if let Some(line) = slot.location() {
            println!("[PCID] {line}");
        }
    }
    let mut held: Vec<(u32, UserDataHandle)> = Vec::new();
    loop {
        match Syscall::ipc_bind(wire::SERVICE_NAME) {
            Ok(binding) => serve(&mut slots, &mut held, binding),
            Err(_) => Syscall::sleep(REBIND_DELAY_MS),
        }
    }
}

fn serve(
    slots: &mut Vec<Slot>,
    held: &mut Vec<(u32, UserDataHandle)>,
    binding: UserBindingHandle,
) {
    loop {
        if let Some(future) = Syscall::ipc_accept_message(binding) {
            if let Ok(received) = Syscall::wait_for_message(future) {
                serve_one(slots, held, &received);
            }
        }
    }
}

fn serve_one(
    slots: &mut Vec<Slot>,
    held: &mut Vec<(u32, UserDataHandle)>,
    received: &usrlib::ipc::ReceivedMessage,
) {
    let Some(connection) = received.connection() else {
        return;
    };
    let owner = conn_token(connection);
    release_held_for(held, owner);
    let reply = match Request::decode(received.data()) {
        Ok(request) => dispatch(slots, held, connection, owner, request),
        Err(error) => Reply::Err { error },
    };
    let Ok(frame) = reply.encode() else {
        return;
    };
    let _ = Syscall::ipc_send(connection, &frame);
}

fn dispatch(
    slots: &mut Vec<Slot>,
    held: &mut Vec<(u32, UserDataHandle)>,
    connection: UserConnectionHandle,
    owner: u32,
    request: Request,
) -> Reply {
    match request {
        Request::Find {
            vendor_id,
            device_id,
        } => match enumeration::find_slot(slots, vendor_id, device_id) {
            Some(slot) => Reply::Slot(*slot),
            None => Reply::Missing,
        },
        Request::List { bus } => {
            let table = wire::encode_slots_table(if bus == SCANNED_BUS {
                slots
            } else {
                &[]
            });
            match Syscall::ipc_data_write(connection, &table) {
                Ok(object) => {
                    let entries = table.len() / wire::SLOT_STRIDE_BYTES;
                    let token = data_token(&object);
                    held.push((owner, object));
                    Reply::List {
                        token,
                        entries: entries as u8,
                    }
                }
                Err(_) => Reply::Err {
                    error: wire::PciError::Protocol,
                },
            }
        }
        Request::Enable {
            bus,
            device,
            function,
        } => match enumeration::enable_io_space(slots, bus, device, function, &mut SyscallConfigBus)
        {
            Some(slot) => {
                if let Some(line) = slot.location() {
                    println!("[PCID] io space enabled: {line}");
                }
                Reply::Slot(slot)
            }
            None => Reply::Missing,
        },
    }
}

fn release_held_for(held: &mut Vec<(u32, UserDataHandle)>, owner: u32) {
    held.retain_mut(|(held_owner, object)| {
        if *held_owner == owner {
            let _ = object.release();
            false
        } else {
            true
        }
    });
}
