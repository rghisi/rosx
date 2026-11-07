#![no_std]

use usrlib::println;
use usrlib::syscall::Syscall;

fn main() {
    match usrlib::args().get(1).copied() {
        None => {}
        Some("panic") => panic!("exiter was told to panic"),
        Some(argument) => match argument.parse::<u32>() {
            Ok(code) => Syscall::exit(code),
            Err(_) => {
                println!("usage: exiter [code|panic]");
                Syscall::exit(2);
            }
        },
    }
}
