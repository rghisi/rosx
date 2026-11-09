#![no_std]

extern crate alloc;

use usrlib::println;
use vfs_protocol::client::Vfs;

mod control;
mod windows;

const DEFAULT_RUN_DIR: &str = "/fsbrun";

fn main() {
    let dir = run_dir();
    println!("bench.start suite=fsbench dir={dir}");
    let Ok(vfs) = Vfs::connect() else {
        println!("fsbench no filesystem service");
        return;
    };
    prepare_run_dir(&vfs, &dir);
    control::run();
    println!("bench.done suite=fsbench");
}

fn run_dir() -> alloc::string::String {
    match usrlib::args().get(1) {
        Some(dir) => alloc::string::String::from(*dir),
        None => alloc::string::String::from(DEFAULT_RUN_DIR),
    }
}

fn prepare_run_dir(vfs: &Vfs, dir: &str) {
    match vfs.mkdir(dir) {
        Ok(()) => println!("fsbench mkdir {dir} ok"),
        Err(error) => println!("fsbench mkdir {dir} kept ({error:?})"),
    }
}
