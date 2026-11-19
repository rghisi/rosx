use usrlib::{print, println};
use usrlib::exit::ExitStatus;
use usrlib::handle::{UserConnectionHandle, UserFutureHandle};
use usrlib::ipc::{ipc_receive_value, ipc_send_value};
use usrlib::syscall::Syscall;
use alloc::string::String;
use alloc::vec::Vec;
use crate::command::Command;
use crate::launcher;
use vfs_protocol::client::{ClientError, Vfs};

static PROMPT: &str = "\x1B[32mrose>\x1B[m ";

struct BackgroundJob {
    name: String,
    future: UserFutureHandle,
}

static COMMANDS: [(&str, fn(&Command)); 5] = [
    ("ls", ls),
    ("clear", clear),
    ("rose", rose),
    ("sleep", sleep),
    ("random", random),
];

fn lookup_command(name: &str) -> Option<fn(&Command)> {
    COMMANDS
        .iter()
        .find(|(command_name, _)| *command_name == name)
        .map(|(_, handler)| *handler)
}

pub fn main() {

    println!("ROSE Shell");
    rose(&Command {
        name: String::new(),
        args: Vec::new(),
        background: false,
    });
    prompt();
    
    let mut buffer = String::new();
    let mut jobs: Vec<BackgroundJob> = Vec::new();
    
    loop {
        let c = Syscall::read_char();
        
        if c == '\n' {
            println!();

            if let Some(cmd) = Command::parse(&buffer) {
                match lookup_command(&cmd.name) {
                    Some(command) => {
                        if cmd.background {
                            println!("Built-in runs in the foreground: {}", cmd.name);
                        }
                        command(&cmd);
                    }
                    None => {
                        if let Some(future) = launcher::run(&cmd) {
                            println!("{} started", cmd.name);
                            jobs.push(BackgroundJob { name: cmd.name.clone(), future });
                        }
                    }
                }
            }
            
            buffer.clear();
            reap_jobs(&mut jobs);
            prompt();
        } else if c == '\x08' {
            if !buffer.is_empty() {
                buffer.pop();
                print!("\x08");
            }
        } else if c == '\x03' {
            println!("^C");
            buffer.clear();
            prompt();
        } else {
            print!("{}", c);
            buffer.push(c);
        }
    }
}

fn reap_jobs(jobs: &mut Vec<BackgroundJob>) {
    let mut index = 0;
    while index < jobs.len() {
        if Syscall::is_future_completed(jobs[index].future) {
            let job = jobs.remove(index);
            match Syscall::wait_for_exit_status(job.future) {
                Ok(status) => report_finished(&job.name, status),
                Err(error) => println!("Cannot watch {}: {}", job.name, error),
            }
        } else {
            index += 1;
        }
    }
}

fn report_finished(name: &str, status: ExitStatus) {
    match status {
        ExitStatus::Clean(0) => println!("{} finished", name),
        ExitStatus::Clean(code) => println!("{} exited with code {}", name, code),
        ExitStatus::Panicked => println!("{} panicked", name),
        ExitStatus::Interrupted => println!("{} interrupted", name),
        ExitStatus::Unknown { reason, code } => {
            println!("{} exited with reason {} code {}", name, reason, code)
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

fn ls(cmd: &Command) {
    match cmd.args.first() {
        None => {
            COMMANDS.iter().for_each(|(command, _)| print!("{}\t", command));
            println!();
            list_through(|vfs| vfs.list("/"));
        }
        Some(path) => list_through(|vfs| vfs.list(path.as_str())),
    }
}

fn list_through(request: impl Fn(&Vfs) -> Result<Vec<String>, ClientError>) {
    match Vfs::connect() {
        Ok(vfs) => {
            match request(&vfs) {
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
                    Ok(_result) => {
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
            println!("Connection failed");
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
