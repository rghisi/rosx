#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::client::{ClientError, Vfs, MODE_CREATE, MODE_WRITE};
use vfs_protocol::wire::VfsError;

const NOTES_BYTES: usize = 9216;
const HOLE_END: usize = 8192;
const BLOB_BYTES: usize = 131072;

fn vfs_byte(file_offset: usize) -> u8 {
    (file_offset as u64).wrapping_mul(29).wrapping_add(11) as u8
}

fn ram_only<'a>(names: &'a [String], baseline: &[String]) -> Vec<&'a str> {
    names
        .iter()
        .map(String::as_str)
        .filter(|name| !baseline.iter().any(|boot| boot == name))
        .collect()
}

fn failed(check: &str) -> bool {
    println!("vfs error: {check}");
    false
}

fn run(vfs: &Vfs) -> bool {
    let Ok(baseline) = vfs.list() else {
        return failed("baseline list");
    };

    let Ok(mut notes) = vfs.open("notes.txt", MODE_WRITE | MODE_CREATE) else {
        return failed("create notes.txt");
    };
    println!("vfs ok: create notes.txt");

    let head: Vec<u8> = (0..4096).map(vfs_byte).collect();
    if notes.write(&head).is_err() || notes.offset() != 4096 {
        return failed("write 4096 at 0");
    }
    println!("vfs ok: write 4096 at 0");
    if notes.close().is_err() {
        return failed("write 4096 at 0");
    }

    let Ok(mut notes) = vfs.open("notes.txt", MODE_WRITE) else {
        return failed("write 1024 at 8192");
    };
    notes.seek(HOLE_END as u64);
    let tail: Vec<u8> = (HOLE_END..HOLE_END + 1024).map(vfs_byte).collect();
    if notes.write(&tail).is_err() || notes.offset() != NOTES_BYTES as u64 {
        return failed("write 1024 at 8192");
    }
    println!("vfs ok: write 1024 at 8192");
    if notes.close().is_err() {
        return failed("write 1024 at 8192");
    }

    let Ok(mut notes) = vfs.open("notes.txt", 0) else {
        return failed("read 9216 hole zeroed");
    };
    let mut whole = vec![0u8; NOTES_BYTES];
    match notes.read(&mut whole) {
        Ok(NOTES_BYTES) => {}
        _ => return failed("read 9216 hole zeroed"),
    }
    if notes.offset() != NOTES_BYTES as u64 {
        return failed("read 9216 hole zeroed");
    }
    for index in 0..4096 {
        if whole[index] != vfs_byte(index) {
            return failed("read 9216 hole zeroed");
        }
    }
    if !whole[4096..HOLE_END].iter().all(|byte| *byte == 0) {
        return failed("read 9216 hole zeroed");
    }
    for index in HOLE_END..NOTES_BYTES {
        if whole[index] != vfs_byte(index) {
            return failed("read 9216 hole zeroed");
        }
    }
    println!("vfs ok: read 9216 hole zeroed");

    match notes.read(&mut [0u8; 16]) {
        Ok(0) => {}
        _ => return failed("eof at 9216"),
    }
    println!("vfs ok: eof at 9216");
    if notes.close().is_err() {
        return failed("eof at 9216");
    }

    let Ok(mut notes) = vfs.open("notes.txt", MODE_WRITE) else {
        return failed("zero write accepted");
    };
    if notes.write(&[]).is_err() {
        return failed("zero write accepted");
    }
    println!("vfs ok: zero write accepted");
    if notes.close().is_err() {
        return failed("zero write accepted");
    }

    let Ok(mut notes) = vfs.open("notes.txt", 0) else {
        return failed("write denied mode 0");
    };
    match notes.write(&[vfs_byte(0)]) {
        Err(ClientError::Vfs(VfsError::NotPermitted)) => {}
        _ => return failed("write denied mode 0"),
    }
    println!("vfs ok: write denied mode 0");
    if notes.close().is_err() {
        return failed("write denied mode 0");
    }

    let Ok(mut notes) = vfs.open("notes.txt", 0) else {
        return failed("reopen kept 9216");
    };
    match notes.read(&mut whole) {
        Ok(NOTES_BYTES) => {}
        _ => return failed("reopen kept 9216"),
    }
    println!("vfs ok: reopen kept 9216");
    if notes.close().is_err() {
        return failed("reopen kept 9216");
    }

    match vfs.list() {
        Ok(names) if ram_only(&names, &baseline) == ["notes.txt"] => {}
        _ => return failed("list got notes.txt"),
    }
    println!("vfs ok: list got notes.txt");

    match vfs.open("notes.txt", MODE_CREATE) {
        Err(ClientError::Vfs(VfsError::AlreadyExists)) => {}
        _ => return failed("create existing denied"),
    }
    println!("vfs ok: create existing denied");

    match vfs.open("missing.txt", 0) {
        Err(ClientError::Vfs(VfsError::NotFound)) => {}
        _ => return failed("open missing denied"),
    }
    println!("vfs ok: open missing denied");

    let Ok(mut blob) = vfs.open("blob.bin", MODE_WRITE | MODE_CREATE) else {
        return failed("chunked write 131072");
    };
    let data: Vec<u8> = (0..BLOB_BYTES).map(vfs_byte).collect();
    let outcome = blob.write(&data);
    drop(data);
    if outcome.is_err() || blob.offset() != BLOB_BYTES as u64 {
        return failed("chunked write 131072");
    }
    println!("vfs ok: chunked write 131072");
    if blob.close().is_err() {
        return failed("chunked write 131072");
    }

    let Ok(mut blob) = vfs.open("blob.bin", 0) else {
        return failed("chunked read 131072 verified");
    };
    let mut readback = vec![0u8; BLOB_BYTES];
    match blob.read(&mut readback) {
        Ok(BLOB_BYTES) => {}
        _ => return failed("chunked read 131072 verified"),
    }
    for index in 0..BLOB_BYTES {
        if readback[index] != vfs_byte(index) {
            return failed("chunked read 131072 verified");
        }
    }
    println!("vfs ok: chunked read 131072 verified");
    if blob.close().is_err() {
        return failed("chunked read 131072 verified");
    }

    match vfs.list() {
        Ok(names) if ram_only(&names, &baseline) == ["blob.bin", "notes.txt"] => {}
        _ => return failed("list got blob.bin notes.txt"),
    }
    println!("vfs ok: list got blob.bin notes.txt");

    if vfs.delete("notes.txt").is_err() {
        return failed("delete notes.txt");
    }
    println!("vfs ok: delete notes.txt");

    match vfs.delete("notes.txt") {
        Err(ClientError::Vfs(VfsError::NotFound)) => {}
        _ => return failed("delete twice denied"),
    }
    println!("vfs ok: delete twice denied");

    match vfs.list() {
        Ok(names) if ram_only(&names, &baseline) == ["blob.bin"] => {}
        _ => return failed("list got blob.bin"),
    }
    println!("vfs ok: list got blob.bin");

    if vfs.delete("blob.bin").is_err() {
        return failed("delete blob.bin");
    }
    println!("vfs ok: delete blob.bin");

    match vfs.list() {
        Ok(names) if ram_only(&names, &baseline).is_empty() => {}
        _ => return failed("list empty"),
    }
    println!("vfs ok: list empty");

    true
}

fn main() {
    println!("[ELF] VFS Client");
    let Ok(vfs) = Vfs::connect() else {
        println!("vfs error: connect");
        return;
    };
    let passed = run(&vfs);
    vfs.disconnect();
    if passed {
        println!("vfs done");
    }
}
