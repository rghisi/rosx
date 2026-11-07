#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use system::boot_plan::{parse_step, Step};
use system::rom::RomImage;
use usrlib::println;
use usrlib::syscall::Syscall;

const PROBE_ATTEMPTS: usize = 100;
const PROBE_DELAY_MS: u64 = 100;

fn main() {
    let rom_bytes = usrlib::rom::read_image();
    let Ok(rom) = RomImage::parse(&rom_bytes) else {
        println!("[INIT] no usable ROM image");
        return;
    };
    let steps = usrlib::args();
    if steps.len() <= 1 {
        println!("[INIT] empty boot plan");
        return;
    }
    println!("[INIT] boot plan of {} steps", steps.len() - 1);
    for line in &steps[1..] {
        match parse_step(line) {
            Err(error) => println!("[INIT] malformed step {line:?}: {error:?}"),
            Ok(Step::Service { name, probe }) => start_service(&rom, name, probe),
            Ok(Step::Run { name, arguments }) => run_tool(&rom, name, &arguments),
            Ok(Step::Exec { name, arguments }) => {
                hand_off(&rom, name, &arguments);
                return;
            }
        }
    }
    println!("[INIT] plan exhausted without an exec step");
}

fn start_service(rom: &RomImage<'_>, name: &str, probe: &str) {
    let Some(bytes) = usrlib::rom::find(rom, name) else {
        println!("[INIT] {name} not in ROM");
        return;
    };
    println!("[INIT] starting {name}");
    if Syscall::load(bytes).is_none() {
        println!("[INIT] {name} cannot launch");
        return;
    }
    await_service(probe);
}

fn await_service(probe: &str) {
    for _ in 0..PROBE_ATTEMPTS {
        if let Ok(connection) = Syscall::ipc_connect(probe) {
            Syscall::ipc_disconnect(connection);
            println!("[INIT] {probe} ready");
            return;
        }
        Syscall::sleep(PROBE_DELAY_MS);
    }
    println!("[INIT] {probe} not ready");
}

fn run_tool(rom: &RomImage<'_>, name: &str, arguments: &[&str]) {
    let Some(bytes) = usrlib::rom::find(rom, name) else {
        println!("[INIT] {name} not in ROM");
        return;
    };
    println!("[INIT] running {name}");
    match Syscall::load_with_args(bytes, &argv(name, arguments)) {
        Some(future) => Syscall::wait_future(future),
        None => println!("[INIT] {name} cannot launch"),
    }
}

fn hand_off(rom: &RomImage<'_>, name: &str, arguments: &[&str]) {
    let Some(bytes) = usrlib::rom::find(rom, name) else {
        println!("[INIT] hand-off target {name} not in ROM");
        return;
    };
    match Syscall::load_with_args(bytes, &argv(name, arguments)) {
        Some(_) => println!("[INIT] handing over to {name}"),
        None => println!("[INIT] {name} cannot launch"),
    }
}

fn argv(name: &str, arguments: &[&str]) -> Vec<String> {
    let mut argv: Vec<String> = Vec::with_capacity(arguments.len() + 1);
    argv.push(String::from(name));
    argv.extend(arguments.iter().map(|argument| String::from(*argument)));
    argv
}
