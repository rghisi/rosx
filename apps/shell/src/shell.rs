use core::arch::asm;
use usrlib::{print, println};
use usrlib::syscall::Syscall;

static mut COUNT: usize = 0;
pub fn main() {
    let c = unsafe { COUNT };
    unsafe {
        COUNT += 1;
    };
    println!("shell {}", c);
    let mut c = 0;
    loop {
        println!("Count: {}", c);
        c += 1;
    }
    // Syscall::exec(main as usize);
    // delay(50000500);
    // Syscall::exec(main as usize);
    println!("closing shell {}", c);
}

fn delay(ticks: u32) {
    for _ in 0..ticks {
        unsafe {
            asm!("nop");
        }
    }
}
