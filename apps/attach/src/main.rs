#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use block_protocol::manager::client::Manager;
use block_protocol::manager::wire::DeviceInfo;
use usrlib::println;

fn main() {
    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };
    let Ok(manager) = Manager::connect() else {
        println!("attach no block device manager");
        return;
    };
    match requested {
        [] => list_devices(&manager),
        [service] => register_provider(&manager, service),
        _ => println!("usage: attach | attach <service>"),
    }
    manager.disconnect();
}

fn register_provider(manager: &Manager, service: &str) {
    match manager.register(service) {
        Err(error) => println!("attach failed: {error:?}"),
        Ok(()) => {
            println!("attach registered {service}");
            list_devices(manager);
        }
    }
}

fn list_devices(manager: &Manager) {
    match manager.list() {
        Err(error) => println!("attach list failed: {error:?}"),
        Ok(devices) if devices.is_empty() => println!("no block devices"),
        Ok(devices) => render_grouped(&devices),
    }
}

fn render_grouped(devices: &[DeviceInfo]) {
    let mut sources: Vec<&str> = Vec::new();
    for device in devices {
        if !sources.contains(&device.source.as_str()) {
            sources.push(&device.source);
        }
    }
    for source in sources {
        println!("{source}:");
        for device in devices.iter().filter(|device| device.source == source) {
            println!(
                "  {}  {}  {}x{}  {}",
                device.name,
                device.description,
                device.block_count,
                device.block_size,
                device.state.label()
            );
        }
    }
}
