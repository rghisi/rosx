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
    let [device, directory] = requested else {
        println!("usage: mount <device-id> <dir>");
        return;
    };
    let name = directory.strip_prefix('/').unwrap_or(directory);
    let Ok(vfs) = Vfs::connect() else {
        println!("mount no filesystem service");
        return;
    };
    match vfs.mount(device, name) {
        Ok(()) => println!("mount ok device={device} dir=/{}", name),
        Err(error) => println!("mount failed: {error:?}"),
    }
    vfs.disconnect();
}
