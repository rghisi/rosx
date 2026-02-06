use usrlib::{print, println};
use usrlib::syscall::Syscall;

pub fn main() {
    println!("RosX Shell");
    print!("> ");
    loop {
        let c = Syscall::read_char();
        print!("{}", c);
        if c == '\n' {
            print!("> ");
        }
    }
}