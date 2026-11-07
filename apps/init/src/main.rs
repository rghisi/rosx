#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use init::image::{is_elf, is_loadable_size, read_stream};
use init::manifest;
use system::rom::RomImage;
use usrlib::println;
use usrlib::syscall::Syscall;
use vfs_protocol::client::{EntityKind, Vfs};

const ROM_READ_CHUNK_BYTES: usize = 4096;

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
    let mut disk = Disk::default();
    for name in &manifest.services {
        load_service(&rom, name, &mut disk);
    }
    handoff(&rom, &manifest, &mut disk);
    disk.disconnect();
}

fn load_service(rom: &RomImage, name: &str, disk: &mut Disk) {
    if let Some(elf) = rom_record(rom, name) {
        if Syscall::load(elf).is_none() {
            println!("[INIT] {name} skipped: load refused");
        }
        return;
    }
    match disk.read_executable(name) {
        Some(image) => {
            if Syscall::load(&image).is_none() {
                println!("[INIT] {name} skipped: load refused");
            }
        }
        None => println!("[INIT] {name} skipped: no disk image"),
    }
}

fn handoff(rom: &RomImage, manifest: &manifest::Manifest, disk: &mut Disk) {
    let Some(target) = &manifest.handoff else {
        return;
    };
    if let Some(image) = disk.read_executable(target) {
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

#[derive(Default)]
struct Disk {
    vfs: Option<Vfs>,
    dialed: bool,
}

impl Disk {
    fn read_executable(&mut self, path: &str) -> Option<Vec<u8>> {
        let vfs = self.vfs()?;
        let entity = vfs.stat(path).ok()?;
        if entity.kind == EntityKind::Directory || !is_loadable_size(entity.size) {
            return None;
        }
        let mut file = vfs.open(path, 0).ok()?;
        let image = read_stream(entity.size, &mut |buf| file.read(buf).ok());
        let _ = file.close();
        image.filter(|image| is_elf(image))
    }

    fn vfs(&mut self) -> Option<&Vfs> {
        if !self.dialed {
            self.dialed = true;
            self.vfs = Vfs::connect().ok();
        }
        self.vfs.as_ref()
    }

    fn disconnect(&mut self) {
        if let Some(vfs) = self.vfs.take() {
            vfs.disconnect();
        }
    }
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
