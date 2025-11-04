use alloc::vec;
use core::arch::asm;
use system::message::{Message, MessageType};
use usrlib::println;
use usrlib::syscall::Syscall;

static mut COUNT: usize = 0;
pub fn main() {
    let c = unsafe { COUNT };
    unsafe { COUNT = COUNT + 1; };
    println!("shell {}", c);
    Syscall::exec(main as usize);
    let message = Message {
        message_type: MessageType::Exec,
        data: vec![1, 0, 0, 19, 136]
    };
    println!("Syscall!!! {}", Syscall::syscall(&message));
    // delay(50000500);
    // Syscall::exec(main as usize);
    println!("closing shell {}", c);
}


fn delay(ticks: u32) {
    for _ in 0..ticks {
        unsafe { asm!("nop"); }
    }
}