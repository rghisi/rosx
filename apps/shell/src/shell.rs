use usrlib::{print, println};
use usrlib::syscall::Syscall;
use alloc::string::String;
use alloc::collections::BTreeMap;
use lazy_static::lazy_static;
use crate::command::Command;

static PI_ELF: &[u8] = include_bytes!("../../../apps/hello_elf/target/rosx-user/release/hello_elf");
static SNAKE_ELF: &[u8] = include_bytes!("../../../apps/snake/target/rosx-user/release/snake");
static TETRIS_ELF: &[u8] = include_bytes!("../../../apps/tetris/target/rosx-user/release/tetris");
static CONWAY_ELF: &[u8] = include_bytes!("../../../apps/conway/target/rosx-user/release/conway");

static PROMPT: &str = "\x1B[32mrose>\x1B[m ";

lazy_static! {
    static ref COMMANDS: BTreeMap<String, fn()> = BTreeMap::from([
        (String::from("ls"), ls as fn()),
        (String::from("clear"), clear as fn()),
        (String::from("rose"), rose as fn()),
        (String::from("pi"), pi as fn()),
        (String::from("snake"), snake as fn()),
        (String::from("tests"), tests as fn()),
        (String::from("tetris"), tetris as fn()),
        (String::from("conway"), conway as fn()),
        (String::from("sleep"), sleep as fn()),
        (String::from("random"), random as fn()),
    ]);
}

pub fn main() {

    println!("ROSE Shell");
    rose();
    prompt();
    
    let mut buffer = String::new();
    
    loop {
        let c = Syscall::read_char();
        
        if c == '\n' {
            println!();

            if let Some(cmd) = Command::parse(&buffer) {
                if let Some(command) = COMMANDS.get(&cmd.name) {
                    command();
                } else {
                    println!("Unknown command: {}", cmd.name);
                }
            }
            
            buffer.clear();
            prompt();
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

fn prompt() {
    print!("{}", PROMPT);
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
    COMMANDS.iter().for_each(|(command, _)| print!("{}\t", command));
    println!();
}

fn pi() {
    Syscall::wait_future(Syscall::load(PI_ELF));
}

fn snake() {
    Syscall::wait_future(Syscall::load(SNAKE_ELF));
}

fn tetris() {
    Syscall::wait_future(Syscall::load(TETRIS_ELF));
}

fn conway() {
    Syscall::wait_future(Syscall::load(CONWAY_ELF));
}

fn tests() {
    Syscall::wait_future(Syscall::exec(test_suite::app::main as usize));
}

fn sleep() {
    Syscall::sleep(1000);
}

fn random() {
    let result = Syscall::ipc_find("RANDOM");
    if let Ok(handle) = result {
        println!("RANDOM Server: {} {}", handle.index, handle.generation);
        for i in 1..100 {
            let random = Syscall::ipc_send(handle, 0);
            println!("RANDOM Value: {}", random.reply.unwrap().value);
        }
    } else {
        println!("Find failed");;
    }
}
