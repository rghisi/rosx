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
    let [path] = requested else {
        println!("usage: rm <path>");
        return;
    };
    let name = path.strip_prefix('/').unwrap_or(path);
    let Ok(vfs) = Vfs::connect() else {
        println!("rm no filesystem service");
        return;
    };
    match vfs.delete(name) {
        Ok(()) => println!("rm ok {path}"),
        Err(error) => println!("rm failed: {error:?}"),
    }
    vfs.disconnect();
}
