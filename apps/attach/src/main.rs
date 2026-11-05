#![no_std]

extern crate alloc;

use usrlib::println;
use vfs_protocol::client::Vfs;

fn main() {
    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };
    let Ok(vfs) = Vfs::connect() else {
        println!("attach no filesystem service");
        return;
    };
    match requested {
        ["list"] => list_devices(&vfs),
        [id, service, device] => attach_device(&vfs, id, service, device),
        _ => println!("usage: attach <id> <service> <device> | attach list"),
    }
    vfs.disconnect();
}

fn attach_device(vfs: &Vfs, id: &str, service: &str, device: &str) {
    let Ok(device) = device.parse::<u32>() else {
        println!("attach wants a numeric device id");
        return;
    };
    match vfs.attach(id, service, device) {
        Ok(()) => println!("attach ok id={id} service={service} device={device}"),
        Err(error) => println!("attach failed: {error:?}"),
    }
}

fn list_devices(vfs: &Vfs) {
    match vfs.list_devices() {
        Ok(devices) => {
            if devices.is_empty() {
                println!("no devices attached");
                return;
            }
            for device in devices {
                println!("{device}");
            }
        }
        Err(error) => println!("attach list failed: {error:?}"),
    }
}
