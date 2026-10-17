#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::panic::PanicInfo;
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::client::{ClientError, MODE_CREATE, MODE_WRITE, Vfs};
use vfs_protocol::wire::VfsError;

struct SyscallAllocator;

unsafe impl GlobalAlloc for SyscallAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Syscall::alloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        Syscall::dealloc(ptr, layout.size(), layout.align());
    }
}

#[global_allocator]
static ALLOCATOR: SyscallAllocator = SyscallAllocator;

const NOTES_BYTES: usize = 9216;
const BLOB_BYTES: usize = 131072;

fn vfs_byte(file_offset: usize) -> u8 {
    (file_offset as u64).wrapping_mul(29).wrapping_add(11) as u8
}

fn pattern(start: usize, len: usize) -> Vec<u8> {
    (start..start + len).map(vfs_byte).collect()
}

fn check(condition: bool, name: &'static str) -> Result<(), &'static str> {
    if condition { Ok(()) } else { Err(name) }
}

fn is_vfs_error<T>(result: Result<T, ClientError>, expected: VfsError) -> bool {
    matches!(result, Err(ClientError::Vfs(error)) if error == expected)
}

fn scenario() -> Result<(), &'static str> {
    let vfs = Vfs::connect().map_err(|_| "connect")?;

    let mut notes = vfs
        .open("notes.txt", MODE_WRITE | MODE_CREATE)
        .map_err(|_| "create notes.txt")?;
    println!("vfs ok: create notes.txt");

    notes
        .write(&pattern(0, 4096))
        .map_err(|_| "write 4096 at 0")?;
    check(notes.offset() == 4096, "write 4096 at 0")?;
    println!("vfs ok: write 4096 at 0");

    notes.close().map_err(|_| "write 1024 at 8192")?;
    let mut sparsely = vfs
        .open("notes.txt", MODE_WRITE)
        .map_err(|_| "write 1024 at 8192")?;
    sparsely.seek(8192);
    sparsely
        .write(&pattern(8192, 1024))
        .map_err(|_| "write 1024 at 8192")?;
    sparsely.close().map_err(|_| "write 1024 at 8192")?;
    println!("vfs ok: write 1024 at 8192");

    let mut reader = vfs
        .open("notes.txt", 0)
        .map_err(|_| "read 9216 hole zeroed")?;
    let mut head = vec![0u8; NOTES_BYTES];
    let delivered = reader
        .read(&mut head)
        .map_err(|_| "read 9216 hole zeroed")?;
    check(delivered == NOTES_BYTES, "read 9216 hole zeroed")?;
    check(
        head[..4096]
            .iter()
            .enumerate()
            .all(|(i, byte)| *byte == vfs_byte(i)),
        "read 9216 hole zeroed",
    )?;
    check(
        head[4096..8192].iter().all(|byte| *byte == 0),
        "read 9216 hole zeroed",
    )?;
    check(
        head[8192..]
            .iter()
            .enumerate()
            .all(|(i, byte)| *byte == vfs_byte(8192 + i)),
        "read 9216 hole zeroed",
    )?;
    check(
        reader.offset() == NOTES_BYTES as u64,
        "read 9216 hole zeroed",
    )?;
    println!("vfs ok: read 9216 hole zeroed");

    let mut probe = [0u8; 16];
    check(
        reader.read(&mut probe).map_err(|_| "eof at 9216")? == 0,
        "eof at 9216",
    )?;
    reader.close().map_err(|_| "eof at 9216")?;
    println!("vfs ok: eof at 9216");

    let mut appender = vfs
        .open("notes.txt", MODE_WRITE)
        .map_err(|_| "zero write accepted")?;
    appender.write(&[]).map_err(|_| "zero write accepted")?;
    appender.close().map_err(|_| "zero write accepted")?;
    println!("vfs ok: zero write accepted");

    let mut readonly = vfs
        .open("notes.txt", 0)
        .map_err(|_| "write denied mode 0")?;
    check(
        is_vfs_error(readonly.write(&[vfs_byte(0)]), VfsError::NotPermitted),
        "write denied mode 0",
    )?;
    readonly.close().map_err(|_| "write denied mode 0")?;
    println!("vfs ok: write denied mode 0");

    let mut kept_reader = vfs.open("notes.txt", 0).map_err(|_| "reopen kept 9216")?;
    let mut kept = vec![0u8; NOTES_BYTES];
    check(
        kept_reader
            .read(&mut kept)
            .map_err(|_| "reopen kept 9216")?
            == NOTES_BYTES,
        "reopen kept 9216",
    )?;
    kept_reader.close().map_err(|_| "reopen kept 9216")?;
    println!("vfs ok: reopen kept 9216");

    check(
        vfs.list().map_err(|_| "list got notes.txt")? == ["notes.txt"],
        "list got notes.txt",
    )?;
    println!("vfs ok: list got notes.txt");

    check(
        is_vfs_error(vfs.open("notes.txt", MODE_CREATE), VfsError::AlreadyExists),
        "create existing denied",
    )?;
    println!("vfs ok: create existing denied");

    check(
        is_vfs_error(vfs.open("missing.txt", 0), VfsError::NotFound),
        "open missing denied",
    )?;
    println!("vfs ok: open missing denied");

    let mut blob = vfs
        .open("blob.bin", MODE_WRITE | MODE_CREATE)
        .map_err(|_| "chunked write 131072")?;
    blob.write(&pattern(0, BLOB_BYTES))
        .map_err(|_| "chunked write 131072")?;
    check(blob.offset() == BLOB_BYTES as u64, "chunked write 131072")?;
    blob.close().map_err(|_| "chunked write 131072")?;
    println!("vfs ok: chunked write 131072");

    let mut blob_reader = vfs
        .open("blob.bin", 0)
        .map_err(|_| "chunked read 131072 verified")?;
    let mut received = vec![0u8; BLOB_BYTES];
    let delivered = blob_reader
        .read(&mut received)
        .map_err(|_| "chunked read 131072 verified")?;
    check(
        delivered == BLOB_BYTES
            && received
                .iter()
                .enumerate()
                .all(|(i, byte)| *byte == vfs_byte(i)),
        "chunked read 131072 verified",
    )?;
    blob_reader
        .close()
        .map_err(|_| "chunked read 131072 verified")?;
    println!("vfs ok: chunked read 131072 verified");

    check(
        vfs.list().map_err(|_| "list got blob.bin notes.txt")? == ["blob.bin", "notes.txt"],
        "list got blob.bin notes.txt",
    )?;
    println!("vfs ok: list got blob.bin notes.txt");

    vfs.delete("notes.txt").map_err(|_| "delete notes.txt")?;
    println!("vfs ok: delete notes.txt");

    check(
        is_vfs_error(vfs.delete("notes.txt"), VfsError::NotFound),
        "delete twice denied",
    )?;
    println!("vfs ok: delete twice denied");

    check(
        vfs.list().map_err(|_| "list got blob.bin")? == ["blob.bin"],
        "list got blob.bin",
    )?;
    println!("vfs ok: list got blob.bin");

    vfs.delete("blob.bin").map_err(|_| "delete blob.bin")?;
    println!("vfs ok: delete blob.bin");

    check(
        vfs.list().map_err(|_| "list empty")?.is_empty(),
        "list empty",
    )?;
    println!("vfs ok: list empty");

    vfs.disconnect();
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    println!("[ELF] VFS Client");
    match scenario() {
        Ok(()) => println!("vfs done"),
        Err(name) => println!("vfs error: {name}"),
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    println!("[VFS Client] Panic!");
    loop {}
}
