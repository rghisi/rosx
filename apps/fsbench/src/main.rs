#![no_std]

extern crate alloc;

use usrlib::println;
use vfs_protocol::client::Vfs;

mod control;
mod dirs;
mod reads;
mod seed;
mod windows;

const DEFAULT_RUN_DIR: &str = "/fsbrun";
const PHASE_READS: u32 = 2;
const PHASE_DIRS: u32 = 3;
const PHASES_ALL: u32 = PHASE_DIRS;

fn main() {
    let arguments = usrlib::args();
    let dir = arguments
        .get(1)
        .map_or(alloc::string::String::from(DEFAULT_RUN_DIR), |arg| alloc::string::String::from(*arg));
    let phases = arguments
        .get(2)
        .and_then(|arg| arg.parse::<u32>().ok())
        .filter(|count| (1..=PHASES_ALL).contains(count))
        .unwrap_or(PHASES_ALL);
    println!("bench.start suite=fsbench dir={dir} phases={phases}");
    let Ok(vfs) = Vfs::connect() else {
        println!("fsbench no filesystem service");
        return;
    };
    prepare_run_dir(&vfs, &dir);
    let big_path = alloc::format!("{dir}/big.bin");
    let wide_path = alloc::format!("{dir}/wide");
    let seeded_reads = phases < PHASE_READS || seed::ensure_big_file(&vfs, &big_path);
    let seeded_dirs = phases < PHASE_DIRS || seed::ensure_wide_dir(&vfs, &wide_path);
    control::run();
    if phases >= PHASE_READS && seeded_reads {
        reads::run(&vfs, &big_path);
    }
    if phases >= PHASE_DIRS && seeded_dirs {
        dirs::run(&vfs, &wide_path);
    }
    println!("bench.done suite=fsbench");
}

fn prepare_run_dir(vfs: &Vfs, dir: &str) {
    match vfs.mkdir(dir) {
        Ok(()) => println!("fsbench mkdir {dir} ok"),
        Err(error) => println!("fsbench mkdir {dir} kept ({error:?})"),
    }
}
