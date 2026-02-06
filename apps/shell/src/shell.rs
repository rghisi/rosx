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
            println!(""); // New line for the command output
            
            if buffer == "ls" {
                println!("Listing Files");
            } else if !buffer.is_empty() {
                println!("Unknown command: {}", buffer);
            }
            
            buffer.clear();
            print!("> ");
        } else if c == '\x08' {
            if !buffer.is_empty() {
                buffer.pop();
                print!("\x08");
            }
        } else {
            print!("{}", c);
            buffer.push(c);
        }
    }
}
