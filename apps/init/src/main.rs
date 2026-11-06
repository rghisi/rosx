#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use init::manifest;
use system::rom::RomImage;
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::client::{EntityKind, Vfs, VfsFile};

const ROM_READ_CHUNK_BYTES: usize = 4096;
const ELF_MAGIC: &[u8; 4] = b"\x7fELF";
const MAX_HANDOFF_BYTES: usize = 8 * 1024 * 1024;

fn main() {
    let image = read_rom_image();
    let Ok(rom) = RomImage::parse(&image) else {
        println!("[INIT] rom image unreadable");
        return;
    };
    let Some(bytes) = rom_record(&rom, manifest::ROM_FILE_NAME) else {
        println!("[INIT] no init.lst record");
        return;
    };
    let (manifest, skipped) = manifest::parse(bytes);
    println!("[INIT] init.lst: {} services", manifest.services.len());
    if skipped > 0 {
        println!("[INIT] {skipped} skipped");
    }
    for name in &manifest.services {
        match rom_record(&rom, name) {
            Some(elf) => {
                if Syscall::load(elf).is_none() {
                    println!("[INIT] {name} skipped: load refused");
                }
            }
            None => println!("[INIT] {name} skipped: no rom record"),
        }
    }
    handoff(&rom, &manifest);
}

fn handoff(rom: &RomImage, manifest: &manifest::Manifest) {
    let Some(target) = &manifest.handoff else {
        return;
    };
    if let Some(image) = read_handoff_image(target) {
        println!("[INIT] handoff {target}");
        if exec(&image, target) {
            return;
        }
    }
    let fallback = manifest
        .fallback
        .as_deref()
        .and_then(|name| rom_record(rom, name).map(|elf| (name, elf)));
    if let Some((name, elf)) = fallback {
        println!("[INIT] fallback {name}");
        if exec(elf, name) {
            return;
        }
    }
    println!("[INIT] handoff failed");
}

fn exec(image: &[u8], name: &str) -> bool {
    let argv = [String::from(name)];
    Syscall::load_with_args(image, &argv).is_some()
}

fn read_rom_image() -> Vec<u8> {
    let mut image = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let mut chunk = [0u8; ROM_READ_CHUNK_BYTES];
        let copied = Syscall::rom_read(offset, &mut chunk);
        if copied == 0 {
            return image;
        }
        image.extend_from_slice(&chunk[..copied]);
        offset += copied as u64;
    }
}

fn rom_record<'a>(rom: &RomImage<'a>, name: &str) -> Option<&'a [u8]> {
    let record = rom.files().find(|file| file.name == name)?;
    Some(&rom.bytes()[record.offset..record.offset + record.len])
}

fn read_handoff_image(target: &str) -> Option<Vec<u8>> {
    let vfs = Vfs::connect().ok()?;
    let entity = match vfs.stat(target) {
        Ok(entity) => entity,
        Err(_) => {
            vfs.disconnect();
            return None;
        }
    };
    if entity.kind == EntityKind::Directory || entity.size as usize > MAX_HANDOFF_BYTES {
        vfs.disconnect();
        return None;
    }
    let mut file = match vfs.open(target, 0) {
        Ok(file) => file,
        Err(_) => {
            vfs.disconnect();
            return None;
        }
    };
    let image = read_whole_file(&mut file, entity.size);
    let _ = file.close();
    vfs.disconnect();
    let image = image?;
    if image.starts_with(ELF_MAGIC) {
        Some(image)
    } else {
        None
    }
}

fn read_whole_file(file: &mut VfsFile, size: u64) -> Option<Vec<u8>> {
    let mut image = Vec::new();
    let mut staging = [0u8; ROM_READ_CHUNK_BYTES];
    while (image.len() as u64) < size {
        let wanted = (size - image.len() as u64).min(ROM_READ_CHUNK_BYTES as u64) as usize;
        match file.read(&mut staging[..wanted]) {
            Ok(0) => return None,
            Ok(count) => image.extend_from_slice(&staging[..count]),
            Err(_) => return None,
        }
    }
    Some(image)
}
