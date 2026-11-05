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
    let Some(device) = requested.first().copied() else {
        println!("usage: format <device-id>");
        return;
    };
    let Ok(vfs) = Vfs::connect() else {
        println!("format no filesystem service");
        return;
    };
    match vfs.format(device) {
        Ok(()) => println!("format ok device={device}"),
        Err(error) => println!("format failed: {error:?}"),
    }
    vfs.disconnect();
}
