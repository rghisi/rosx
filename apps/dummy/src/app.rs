use core::arch::asm;
use usrlib::print;

pub fn main() {
    print!("1");
    for _ in 0..10 {
        delay(20000500);
        print!("1");
    }
    print!("1");
}

pub fn main2() {
    print!("2");
    for _ in 0..10 {
        delay(20000500);
        print!("2");
    }
    print!("2");
}
pub fn main3() {
    print!("3");
    for _ in 0..10 {
        delay(20000500);
        print!("3");
    }
    print!("3");
}
pub fn main4() {
    print!("4");
    for _ in 0..10 {
        delay(20000500);
        print!("4");
    }
    print!("4");
}
fn delay(ticks: u32) {
    for _ in 0..ticks {
        unsafe { asm!("nop"); }
    }
}