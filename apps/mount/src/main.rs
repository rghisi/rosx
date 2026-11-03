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
    let [id, directory] = requested else {
        println!("usage: mount <disk-id> <dir>");
        return;
    };
    let Ok(drive) = id.parse::<u8>() else {
        println!("mount wants a numeric disk id");
        return;
    };
    let name = directory.strip_prefix('/').unwrap_or(directory);
    let Ok(vfs) = Vfs::connect() else {
        println!("mount no filesystem service");
        return;
    };
    match vfs.mount(drive, name) {
        Ok(()) => println!("mount ok drive={drive} dir={directory}"),
        Err(error) => println!("mount failed: {error:?}"),
    }
    vfs.disconnect();
}
