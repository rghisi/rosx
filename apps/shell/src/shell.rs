use usrlib::{print, println};
use usrlib::syscall::Syscall;
use alloc::string::String;

pub fn main() {
    println!("RosX Shell");
    print!("> ");
    
    let mut buffer = String::new();
    
    loop {
        let c = Syscall::read_char();
        
        if c == '\n' {
            println!();
            
            if buffer == "ls" {
                println!("Listing Files");
            } else if !buffer.is_empty() {
                println!("Unknown command: {}", buffer);
            }
            
            buffer.clear();
            print!("> ");
        } else {
            print!("{}", c);
            buffer.push(c);
        }
    }
}