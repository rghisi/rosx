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
        println!("usage: mkdir <path>");
        return;
    };
    let Ok(vfs) = Vfs::connect() else {
        println!("mkdir no filesystem service");
        return;
    };
    match vfs.mkdir(path) {
        Ok(()) => println!("mkdir ok {path}"),
        Err(error) => println!("mkdir failed: {error:?}"),
    }
    vfs.disconnect();
}
