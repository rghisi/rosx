#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use usrlib::println;
use vfs_protocol::client::{Vfs, MODE_CREATE, MODE_WRITE};

const READ_CHUNK_BYTES: usize = 4096;

fn main() {
    let arguments = usrlib::args();
    let requested: &[&str] = if arguments.is_empty() {
        &[]
    } else {
        &arguments[1..]
    };
    let [source, target] = requested else {
        println!("usage: cp <source> <target>");
        return;
    };
    let Ok(vfs) = Vfs::connect() else {
        println!("cp no filesystem service");
        return;
    };
    let mut reader = match vfs.open(source, 0) {
        Ok(file) => file,
        Err(error) => {
            println!("cp cannot read {source}: {error:?}");
            return;
        }
    };
    let mut contents: Vec<u8> = Vec::new();
    loop {
        let mut chunk = [0u8; READ_CHUNK_BYTES];
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => contents.extend_from_slice(&chunk[..read]),
            Err(error) => {
                println!("cp failed reading {source}: {error:?}");
                return;
            }
        }
    }
    if let Err(error) = reader.close() {
        println!("cp failed closing {source}: {error:?}");
        return;
    }
    let mut writer = match vfs.open(target, MODE_CREATE | MODE_WRITE) {
        Ok(file) => file,
        Err(error) => {
            println!("cp cannot write {target}: {error:?}");
            return;
        }
    };
    if let Err(error) = writer.write(&contents) {
        println!("cp failed writing {target}: {error:?}");
        return;
    }
    if let Err(error) = writer.close() {
        println!("cp failed closing {target}: {error:?}");
        return;
    }
    println!("cp ok {source} -> {target} {} bytes", contents.len());
    vfs.disconnect();
}
