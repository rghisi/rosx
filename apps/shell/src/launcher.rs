use crate::command::Command;
use alloc::format;
use alloc::vec::Vec;
use input_protocol::wire;
use system::args::{MAX_ARGUMENT_BYTES, MAX_ARGUMENT_COUNT};
use usrlib::exit::ExitStatus;
use usrlib::handle::{UserConnectionHandle, UserFutureHandle};
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::client::{ClientError, EntityKind, Vfs, VfsFile};
use vfs_protocol::wire::VfsError;

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const READ_CHUNK_BYTES: usize = 4096;
const CHILD_EVENT_TAG: u32 = 1;
const CONTROL_EVENT_TAG: u32 = 2;

pub fn run(cmd: &Command) -> Option<UserFutureHandle> {
    let vfs = match Vfs::connect() {
        Ok(vfs) => vfs,
        Err(error) => {
            println!("No filesystem: {:?}", error);
            return None;
        }
    };
    let program_path = format!("/{}", cmd.name);
    let entity = match vfs.stat(&program_path) {
        Ok(entity) => entity,
        Err(ClientError::Vfs(VfsError::NotFound)) => {
            println!("Unknown command: {}", cmd.name);
            vfs.disconnect();
            return None;
        }
        Err(error) => {
            println!("Cannot stat {}: {:?}", cmd.name, error);
            vfs.disconnect();
            return None;
        }
    };
    if entity.kind == EntityKind::Directory {
        println!("Not an executable: {}", cmd.name);
        vfs.disconnect();
        return None;
    }
    let mut file = match vfs.open(program_path.as_str(), 0) {
        Ok(file) => file,
        Err(ClientError::Vfs(VfsError::NotFound)) => {
            println!("Unknown command: {}", cmd.name);
            vfs.disconnect();
            return None;
        }
        Err(error) => {
            println!("Cannot open {}: {:?}", cmd.name, error);
            vfs.disconnect();
            return None;
        }
    };
    let image = read_whole_file(&mut file, &cmd.name, entity.size);
    let _ = file.close();
    vfs.disconnect();
    let Some(image) = image else { return None };
    if !image.starts_with(ELF_MAGIC) {
        println!("Not an executable: {}", cmd.name);
        return None;
    }
    launch(image, cmd)
}

fn read_whole_file(file: &mut VfsFile, name: &str, size: u64) -> Option<Vec<u8>> {
    let mut image: Vec<u8> = Vec::new();
    let mut staging = [0u8; READ_CHUNK_BYTES];
    while (image.len() as u64) < size {
        let wanted = (size - image.len() as u64).min(READ_CHUNK_BYTES as u64) as usize;
        match file.read(&mut staging[..wanted]) {
            Ok(0) => {
                println!("Cannot read {}: truncated image", name);
                return None;
            }
            Ok(count) => image.extend_from_slice(&staging[..count]),
            Err(error) => {
                println!("Cannot read {}: {:?}", name, error);
                return None;
            }
        }
    }
    Some(image)
}

fn launch(image: Vec<u8>, cmd: &Command) -> Option<UserFutureHandle> {
    let argument_count = cmd.args.len() + 1;
    if argument_count > MAX_ARGUMENT_COUNT {
        println!("Too many arguments: maximum is {}", MAX_ARGUMENT_COUNT);
        return None;
    }
    let encoded_size =
        cmd.name.len() + cmd.args.iter().map(|arg| arg.len()).sum::<usize>() + argument_count;
    if encoded_size > MAX_ARGUMENT_BYTES {
        println!(
            "Arguments too large: maximum is {} bytes",
            MAX_ARGUMENT_BYTES
        );
        return None;
    }
    let mut argv = Vec::with_capacity(argument_count);
    argv.push(cmd.name.clone());
    argv.extend(cmd.args.iter().cloned());
    match Syscall::load_with_args(&image, &argv) {
        Some(handle) if cmd.background => Some(handle),
        Some(handle) => {
            wait_for_foreground(handle, &cmd.name);
            None
        }
        None => {
            println!("Cannot launch: {}", cmd.name);
            None
        }
    }
}

struct ControlWatcher {
    connection: UserConnectionHandle,
}

impl ControlWatcher {
    fn open() -> Option<ControlWatcher> {
        let connection = Syscall::ipc_connect(wire::SERVICE_NAME).ok()?;
        if Syscall::ipc_send(connection, &wire::encode_watch()).is_err() {
            Syscall::ipc_disconnect(connection);
            return None;
        }
        Some(ControlWatcher { connection })
    }

    fn close(&self) {
        let _ = Syscall::ipc_send(self.connection, &wire::encode_unwatch());
        Syscall::ipc_disconnect(self.connection);
    }
}

fn wait_for_foreground(child: UserFutureHandle, name: &str) {
    let Some(watcher) = ControlWatcher::open() else {
        report_wait(child, name);
        return;
    };
    let Some(control) = Syscall::ipc_receive_message(watcher.connection) else {
        watcher.close();
        report_wait(child, name);
        return;
    };
    let Ok(set) = Syscall::waitset_create() else {
        watcher.close();
        report_wait(child, name);
        return;
    };
    if Syscall::waitset_attach(set, child, CHILD_EVENT_TAG).is_err()
        || Syscall::waitset_attach(set, control, CONTROL_EVENT_TAG).is_err()
    {
        Syscall::waitset_drop(set);
        watcher.close();
        report_wait(child, name);
        return;
    }
    let mut receive_still_pending = true;
    loop {
        let Ok((completed, tag)) = Syscall::waitset_wait(set) else { break };
        if tag == CHILD_EVENT_TAG {
            break;
        }
        if tag == CONTROL_EVENT_TAG && completed == control {
            let is_interrupt = Syscall::wait_for_message(control)
                .map(|message| wire::frame_tag(message.data()) == wire::TAG_CONTROL)
                .unwrap_or(false);
            receive_still_pending = false;
            if is_interrupt {
                println!("^C");
                let _ = Syscall::kill(child);
            }
        }
    }
    Syscall::waitset_drop(set);
    let unwatch_sent = Syscall::ipc_send(watcher.connection, &wire::encode_unwatch()).is_ok();
    if receive_still_pending && unwatch_sent {
        let _ = Syscall::wait_for_message(control);
    }
    Syscall::ipc_disconnect(watcher.connection);
    report_wait(child, name);
}

fn report_wait(child: UserFutureHandle, name: &str) {
    match Syscall::wait_for_exit_status(child) {
        Ok(status) => report_exit(name, status),
        Err(error) => println!("Cannot watch {}: {}", name, error),
    }
}

fn report_exit(name: &str, status: ExitStatus) {
    match status {
        ExitStatus::Clean(0) => {}
        ExitStatus::Clean(code) => println!("{} exited with code {}", name, code),
        ExitStatus::Panicked => println!("{} panicked", name),
        ExitStatus::Interrupted => {}
        ExitStatus::Unknown { reason, code } => {
            println!("{} exited with reason {} code {}", name, reason, code)
        }
    }
}
