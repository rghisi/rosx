#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use system::boot_plan::{parse_program, parse_step, Program, Step};
use system::rom::RomImage;
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::client::Vfs;

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
                if hand_off(&rom, name, &arguments) {
                    return;
                }
            }
        }
    }
    println!("[INIT] plan exhausted without a successful hand-off");
}

fn program_image(rom: &RomImage<'_>, token: &str) -> Option<Vec<u8>> {
    match parse_program(token) {
        Some(Program::Rom(name)) | Some(Program::ImplicitRom(name)) => {
            usrlib::rom::find(rom, name).map(|bytes| Vec::from(bytes))
        }
        Some(Program::Vfs(path)) => read_from_vfs(path),
        None => None,
    }
}

fn read_from_vfs(path: &str) -> Option<Vec<u8>> {
    let Ok(vfs) = Vfs::connect() else {
        println!("[INIT] {path} no filesystem service");
        return None;
    };
    let image = vfs.read_whole(path);
    vfs.disconnect();
    match image {
        Ok(image) => Some(image),
        Err(error) => {
            println!("[INIT] {path} cannot read: {error:?}");
            None
        }
    }
}

fn start_service(rom: &RomImage<'_>, token: &str, probe: &str) {
    let Some(image) = program_image(rom, token) else {
        println!("[INIT] {token} not available");
        return;
    };
    println!("[INIT] starting {token}");
    if Syscall::load_with_args(&image, &argv(token, &[])).is_none() {
        println!("[INIT] {token} cannot launch");
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

fn run_tool(rom: &RomImage<'_>, token: &str, arguments: &[&str]) {
    let Some(image) = program_image(rom, token) else {
        println!("[INIT] {token} not available");
        return;
    };
    println!("[INIT] running {token}");
    match Syscall::load_with_args(&image, &argv(token, arguments)) {
        Some(future) => Syscall::wait_future(future),
        None => println!("[INIT] {token} cannot launch"),
    }
}

fn hand_off(rom: &RomImage<'_>, token: &str, arguments: &[&str]) -> bool {
    let Some(image) = program_image(rom, token) else {
        println!("[INIT] hand-off {token} unavailable");
        return false;
    };
    match Syscall::load_with_args(&image, &argv(token, arguments)) {
        Some(_) => {
            println!("[INIT] handing over to {token}");
            true
        }
        None => {
            println!("[INIT] {token} cannot launch");
            false
        }
    }
}

fn argv(token: &str, arguments: &[&str]) -> Vec<String> {
    let label = parse_program(token)
        .map(|program| program.name())
        .unwrap_or(token);
    let mut argv: Vec<String> = Vec::with_capacity(arguments.len() + 1);
    argv.push(String::from(label));
    argv.extend(arguments.iter().map(|argument| String::from(*argument)));
    argv
}
