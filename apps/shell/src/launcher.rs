use crate::command::Command;
use alloc::vec::Vec;
use system::args::{MAX_ARGUMENT_BYTES, MAX_ARGUMENT_COUNT};
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::client::{ClientError, Vfs, VfsFile};
use vfs_protocol::wire::VfsError;

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const READ_CHUNK_BYTES: usize = 4096;

pub fn run(cmd: &Command) {
    let vfs = match Vfs::connect() {
        Ok(vfs) => vfs,
        Err(error) => {
            println!("No filesystem: {:?}", error);
            return;
        }
    };
    let mut file = match vfs.open(cmd.name.as_str(), 0) {
        Ok(file) => file,
        Err(ClientError::Vfs(VfsError::NotFound)) => {
            println!("Unknown command: {}", cmd.name);
            vfs.disconnect();
            return;
        }
        Err(error) => {
            println!("Cannot open {}: {:?}", cmd.name, error);
            vfs.disconnect();
            return;
        }
    };
    let image = read_whole_file(&mut file, &cmd.name);
    let _ = file.close();
    vfs.disconnect();
    let Some(image) = image else { return };
    if !image.starts_with(ELF_MAGIC) {
        println!("Not an executable: {}", cmd.name);
        return;
    }
    launch(image, cmd);
}

fn read_whole_file(file: &mut VfsFile, name: &str) -> Option<Vec<u8>> {
    let mut image: Vec<u8> = Vec::new();
    let mut staging = [0u8; READ_CHUNK_BYTES];
    loop {
        match file.read(&mut staging) {
            Ok(0) => return Some(image),
            Ok(count) => image.extend_from_slice(&staging[..count]),
            Err(error) => {
                println!("Cannot read {}: {:?}", name, error);
                return None;
            }
        }
    }
}

fn launch(image: Vec<u8>, cmd: &Command) {
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
    match Syscall::load_with_args(&image, &argv) {
        Some(handle) => Syscall::wait_future(handle),
        None => println!("Cannot launch: {}", cmd.name),
    }
}
