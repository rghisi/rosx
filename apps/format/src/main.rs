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
    let Some(id) = requested.first().copied() else {
        println!("usage: format <disk-id>");
        return;
    };
    let Ok(drive) = id.parse::<u8>() else {
        println!("format wants a numeric disk id");
        return;
    };
    let Ok(vfs) = Vfs::connect() else {
        println!("format no filesystem service");
        return;
    };
    match vfs.format(drive) {
        Ok(()) => println!("format ok drive={drive}"),
        Err(error) => println!("format failed: {error:?}"),
    }
    vfs.disconnect();
}
