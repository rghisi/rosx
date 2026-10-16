use usrlib::{print, println};
use usrlib::handle::UserConnectionHandle;
use usrlib::ipc::{ipc_receive_value, ipc_send_value};
use usrlib::syscall::Syscall;
use alloc::string::String;
use alloc::collections::BTreeMap;
use lazy_static::lazy_static;
use crate::command::Command;

#[cfg(target_arch = "x86_64")]
static PI_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/hello_elf");
#[cfg(target_arch = "x86")]
static PI_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/hello_elf");

#[cfg(target_arch = "x86_64")]
static SNAKE_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/snake");
#[cfg(target_arch = "x86")]
static SNAKE_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/snake");

#[cfg(target_arch = "x86_64")]
static TETRIS_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/tetris");
#[cfg(target_arch = "x86")]
static TETRIS_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/tetris");

#[cfg(target_arch = "x86_64")]
static CONWAY_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/conway");
#[cfg(target_arch = "x86")]
static CONWAY_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/conway");

#[cfg(target_arch = "x86_64")]
static PING_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/ping_client");

static PROMPT: &str = "\x1B[32mrose>\x1B[m ";

lazy_static! {
    static ref COMMANDS: BTreeMap<String, fn()> = {
        #[allow(unused_mut)]
        let mut commands: BTreeMap<String, fn()> = BTreeMap::from([
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
        #[cfg(target_arch = "x86_64")]
        commands.insert(String::from("ping"), ping as fn());
        commands
    };
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
    if let Some(fh) = Syscall::load(PI_ELF) {
        Syscall::wait_future(fh);
    }
}

fn snake() {
    if let Some(fh) = Syscall::load(SNAKE_ELF) {
        Syscall::wait_future(fh);
    }
}

fn tetris() {
    if let Some(fh) = Syscall::load(TETRIS_ELF) {
        Syscall::wait_future(fh);
    }
}

fn conway() {
    if let Some(fh) = Syscall::load(CONWAY_ELF) {
        Syscall::wait_future(fh);
    }
}

#[cfg(target_arch = "x86_64")]
fn ping() {
    if let Some(fh) = Syscall::load(PING_ELF) {
        Syscall::wait_future(fh);
    }
}

fn tests() {
    if let Some(fh) = Syscall::exec(test_suite::app::main as usize) {
        Syscall::wait_future(fh);
    }
}

fn sleep() {
    Syscall::sleep(1000);
}

fn random() {
    let mut first: Option<UserConnectionHandle> = None;
    for _ in 1..260 {
        let ipc_connection_result = Syscall::ipc_connect("RANDOM");
        if let Ok(ipc_connection) = ipc_connection_result {
            if first.is_none() {
                first = Some(ipc_connection);
            }
            println!("RANDOM Server: {}", ipc_connection);
            for _ in 1..2 {
                match ipc_send_value(ipc_connection, 123456) {
                    Ok(result) => {
                        println!("RANDOM Value requested");
                        match ipc_receive_value(ipc_connection) {
                            Ok(value) => println!("RANDOM Value: {}", value),
                            Err(e) => println!("RANDOM Value not received: {}", e),
                        }
                    },
                    Err(result) => {
                        println!("RANDOM Failed to send: {}", result);
                    }
                }
            }
            Syscall::ipc_disconnect(ipc_connection);
        } else {
            println!("Connection failed");;
        }
    }
    if let Some(stale) = first {
        match ipc_send_value(stale, 0) {
            Err(_) => println!("RANDOM Stale rejected"),
            Ok(()) => println!("RANDOM Stale accepted"),
        }
        if let Ok(fresh) = Syscall::ipc_connect("RANDOM") {
            match ipc_send_value(fresh, 123456) {
                Ok(_) => match ipc_receive_value(fresh) {
                    Ok(value) => println!("RANDOM Fresh value: {}", value),
                    Err(e) => println!("RANDOM Value not received: {}", e),
                },
                Err(result) => {
                    println!("RANDOM Failed to send: {}", result);
                }
            }
            Syscall::ipc_disconnect(fresh);
        } else {
            println!("Connection failed");
        }
    }
}
