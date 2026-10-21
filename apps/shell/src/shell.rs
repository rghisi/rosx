use usrlib::{print, println};
use usrlib::handle::UserConnectionHandle;
use usrlib::ipc::{ipc_receive_value, ipc_send_value};
use usrlib::syscall::Syscall;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use lazy_static::lazy_static;
use crate::command::Command;
use crate::launcher;
use vfs_protocol::client::Vfs;

static PROMPT: &str = "\x1B[32mrose>\x1B[m ";

lazy_static! {
    static ref COMMANDS: BTreeMap<String, fn(&Command)> = BTreeMap::from([
        (String::from("ls"), ls as fn(&Command)),
        (String::from("clear"), clear as fn(&Command)),
        (String::from("rose"), rose as fn(&Command)),
        (String::from("tests"), tests as fn(&Command)),
        (String::from("sleep"), sleep as fn(&Command)),
        (String::from("random"), random as fn(&Command)),
    ]);
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
                match COMMANDS.get(&cmd.name) {
                    Some(command) => command(&cmd),
                    None => launcher::run(&cmd),
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
    match Vfs::connect() {
        Ok(vfs) => {
            match vfs.list() {
                Ok(names) => {
                    if !names.is_empty() {
                        names.iter().for_each(|name| print!("{}\t", name));
                        println!();
                    }
                }
                Err(error) => println!("Cannot list files: {:?}", error),
            }
            vfs.disconnect();
        }
        Err(error) => println!("No filesystem: {:?}", error),
    }
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
