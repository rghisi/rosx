use usrlib::{print, println};
use usrlib::handle::UserConnectionHandle;
use usrlib::ipc::{ipc_receive_value, ipc_send_value};
use usrlib::syscall::Syscall;
use system::args::{MAX_ARGUMENT_BYTES, MAX_ARGUMENT_COUNT};
use alloc::string::String;
use alloc::vec::Vec;
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

#[cfg(target_arch = "x86_64")]
static VFS_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/vfs_client");

static PROMPT: &str = "\x1B[32mrose>\x1B[m ";

lazy_static! {
    static ref COMMANDS: BTreeMap<String, fn(&Command)> = {
        #[allow(unused_mut)]
        let mut commands: BTreeMap<String, fn(&Command)> = BTreeMap::from([
            (String::from("ls"), ls as fn(&Command)),
            (String::from("clear"), clear as fn(&Command)),
            (String::from("rose"), rose as fn(&Command)),
            (String::from("pi"), pi as fn(&Command)),
            (String::from("snake"), snake as fn(&Command)),
            (String::from("tests"), tests as fn(&Command)),
            (String::from("tetris"), tetris as fn(&Command)),
            (String::from("conway"), conway as fn(&Command)),
            (String::from("sleep"), sleep as fn(&Command)),
            (String::from("random"), random as fn(&Command)),
        ]);
        #[cfg(target_arch = "x86_64")]
        commands.insert(String::from("ping"), ping as fn(&Command));
        #[cfg(target_arch = "x86_64")]
        commands.insert(String::from("vfs"), vfs as fn(&Command));
        commands
    };
}

pub fn main() {

    println!("ROSE Shell");
    rose(&Command {
        name: String::new(),
        args: Vec::new(),
    });
    prompt();
    
    let mut buffer = String::new();
    
    loop {
        let c = Syscall::read_char();
        
        if c == '\n' {
            println!();

            if let Some(cmd) = Command::parse(&buffer) {
                if let Some(command) = COMMANDS.get(&cmd.name) {
                    command(&cmd);
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

fn spawn(elf: &'static [u8], cmd: &Command) {
    let argument_count = cmd.args.len() + 1;
    if argument_count > MAX_ARGUMENT_COUNT {
        println!("Too many arguments: maximum is {}", MAX_ARGUMENT_COUNT);
        return;
    }
    let encoded_size =
        cmd.name.len() + cmd.args.iter().map(|arg| arg.len()).sum::<usize>() + argument_count;
    if encoded_size > MAX_ARGUMENT_BYTES {
        println!(
            "Arguments too large: maximum is {} bytes",
            MAX_ARGUMENT_BYTES
        );
        return;
    }
    let mut argv = Vec::with_capacity(argument_count);
    argv.push(cmd.name.clone());
    argv.extend(cmd.args.iter().cloned());
    if let Some(fh) = Syscall::load_with_args(elf, &argv) {
        Syscall::wait_future(fh);
    }
}

fn rose(_cmd: &Command) {
    println!("\x1B[40m\x1B[31m       _");
    println!("\x1B[40m\x1B[31m     _( )_");
    println!("\x1B[40m\x1B[31m    (_(%)_)    \x1B[32m_");
    println!("\x1B[40m\x1B[31m      (_)\x1B[32m     (_)");
    println!("\x1B[40m\x1B[32m        |    _//");
    println!("\x1B[40m\x1B[32m         \\  //");
    println!("\x1B[40m\x1B[32m          \\//");
    println!("\x1B[40m\x1B[32m           |");
}

fn clear(_cmd: &Command) {
    print!("\x1B[2J\x1B[H");
}

fn ls(_cmd: &Command) {
    COMMANDS.iter().for_each(|(command, _)| print!("{}\t", command));
    println!();
}

fn pi(cmd: &Command) {
    spawn(PI_ELF, cmd);
}

fn snake(cmd: &Command) {
    spawn(SNAKE_ELF, cmd);
}

fn tetris(cmd: &Command) {
    spawn(TETRIS_ELF, cmd);
}

fn conway(cmd: &Command) {
    spawn(CONWAY_ELF, cmd);
}

#[cfg(target_arch = "x86_64")]
fn ping(cmd: &Command) {
    spawn(PING_ELF, cmd);
}

#[cfg(target_arch = "x86_64")]
fn vfs(cmd: &Command) {
    spawn(VFS_ELF, cmd);
}

fn tests(_cmd: &Command) {
    if let Some(fh) = Syscall::exec(test_suite::app::main as usize) {
        Syscall::wait_future(fh);
    }
}

fn sleep(_cmd: &Command) {
    Syscall::sleep(1000);
}

fn random(_cmd: &Command) {
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
