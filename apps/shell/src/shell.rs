use usrlib::{print, println};
use usrlib::syscall::Syscall;
use alloc::string::String;

pub fn main() {
    println!("RosX Shell");
    print!("\x1B[32m>\x1B[m ");
    
    let mut buffer = String::new();
    
    loop {
        let c = Syscall::read_char();
        
        if c == '\n' {
            println!(""); // New line for the command output
            
            if buffer == "ls" {
                println!("Listing Files");
            } else if buffer == "clear" {
                print!("\x1B[2J\x1B[H");
            } else if buffer == "rose" {
                println!("\x1B[40m\x1B[31m       _");
                println!("\x1B[40m\x1B[31m     _( )_");
                println!("\x1B[40m\x1B[31m    (_(%)_)    \x1B[32m_");
                println!("\x1B[40m\x1B[31m      (_)\x1B[32m     (_)");
                println!("\x1B[40m\x1B[32m        |    _//");
                println!("\x1B[40m\x1B[32m         \\  //");
                println!("\x1B[40m\x1B[32m          \\//");
                println!("\x1B[40m\x1B[32m           |");
            } else if !buffer.is_empty() {
                println!("Unknown command: {}", buffer);
            }
            
            buffer.clear();
            print!("\x1B[32m>\x1B[m ");
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
