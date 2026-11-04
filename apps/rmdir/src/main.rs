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
        println!("usage: rmdir <path>");
        return;
    };
    let name = path.strip_prefix('/').unwrap_or(path);
    let Ok(vfs) = Vfs::connect() else {
        println!("rmdir no filesystem service");
        return;
    };
    match vfs.rmdir(name) {
        Ok(()) => println!("rmdir ok {path}"),
        Err(error) => println!("rmdir failed: {error:?}"),
    }
    vfs.disconnect();
}
