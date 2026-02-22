use usrlib::print;
use usrlib::syscall::Syscall;

pub fn main() {
    print!("1");
    for _ in 0..10 {
        Syscall::sleep(10);
        print!("1");
    }
    print!("1");
}

pub fn main2() {
    print!("2");
    for _ in 0..10 {
        Syscall::sleep(20);
        print!("2");
    }
    print!("2");
}
pub fn main3() {
    print!("3");
    for _ in 0..10 {
        Syscall::sleep(30);
        print!("3");
    }
    print!("3");
}
pub fn main4() {
    print!("4");
    for _ in 0..10 {
        Syscall::sleep(40);
        print!("4");
    }
    print!("4");
}

pub fn main_with_wait() {
    print!("Task that will wait 2s: 0, ");
    Syscall::sleep(1000);
    print!("1, ");
    Syscall::sleep(1000);
    print!("2. Done");
}

