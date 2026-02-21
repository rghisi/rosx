use usrlib::{print, println};
use usrlib::syscall::Syscall;
use alloc::string::String;
use alloc::collections::BTreeMap;
use lazy_static::lazy_static;

static PI_ELF: &[u8] = include_bytes!("../../../apps/hello_elf/target/rosx-user/release/hello_elf");
static SNAKE_ELF: &[u8] = include_bytes!("../../../apps/snake/target/rosx-user/release/snake");

lazy_static! {
    static ref COMMANDS: BTreeMap<String, fn()> = BTreeMap::from([
        (String::from("ls"), ls as fn()),
        (String::from("clear"), clear as fn()),
        (String::from("rose"), rose as fn()),
        (String::from("pi"), pi as fn()),
        (String::from("snake"), snake as fn()),
        (String::from("tests"), tests as fn()),
    ]);
}

pub fn main() {

    println!("RosX Shell");
    print!("\x1B[32m>\x1B[m ");
    
    let mut buffer = String::new();
    
    loop {
        let c = Syscall::read_char();
        
        if c == '\n' {
            println!();

            if let Some(command) = COMMANDS.get(&buffer) {
                command();
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

fn rose() {
    println!("\x1B[40m\x1B[31m       _");
    println!("\x1B[40m\x1B[31m     _( )_");
    println!("\x1B[40m\x1B[31m    (_(%)_)    \x1B[32m_");
    println!("\x1B[40m\x1B[31m      (_)\x1B[32m     (_)");
    println!("\x1B[40m\x1B[32m        |    _//");
    println!("\x1B[40m\x1B[32m         \\  //");
    println!("\x1B[40m\x1B[32m          \\//");
    println!("\x1B[40m\x1B[32m           |");
}

fn clear() {
    print!("\x1B[2J\x1B[H");
}

fn ls() {
    COMMANDS.iter().for_each(|(command, _)| println!("{}", command));
}

fn pi() {
    Syscall::wait_future(Syscall::load(PI_ELF));
}

fn snake() {
    Syscall::wait_future(Syscall::load(SNAKE_ELF));
}

fn tests() {
    Syscall::wait_future(Syscall::exec(test_suite::app::main as usize));
}
