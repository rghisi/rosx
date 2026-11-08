use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use rxpart::mbr::{Entry, ENTRY_COUNT};

use crate::arch::ArchSpec;
use crate::arch::USER_APPS;
use crate::{check_isa, run, workspace_root};

const SECTOR_BYTES: u64 = 512;
const HEADS: &str = "16";
const SECTORS_PER_TRACK: &str = "63";
const PARTITION_START_LBA: u32 = 63;
const BOOTABLE: u8 = 0x80;
const FAT16_TYPE: u8 = 0x06;
const BOOT_DISK_BYTES: u64 = 64 * 1024 * 1024;
const VOLUME_LABEL: &str = "ROSXBOOT";
const MAX_FAT_NAME_BYTES: usize = 8;

pub fn boot_disk_path(spec: &ArchSpec) -> PathBuf {
    workspace_root()
        .join("target")
        .join(format!("{}-boot-disk.img", spec.kernel_dir_name))
}

pub fn create_boot_disk(debug: bool, spec: &ArchSpec) -> PathBuf {
    require_fat_tools();
    require_names_fit_8_3();
    let path = boot_disk_path(spec);
    let partition_sectors = stamp_mbr(&path);
    mformat_volume(&path, partition_sectors);
    mmake_boot_directory(&path);
    mcopy_apps(&path, &spec.user_release_dir());
    let kernel = spec.kernel_elf(if debug { "debug" } else { "release" });
    mcopy_boot_files(&path, &kernel, &spec.rom_path());
    println!("==> wrote FAT boot disk {}", path.display());
    path
}

fn require_fat_tools() {
    let missing: Vec<&str> = ["mformat", "mmd", "mcopy"]
        .into_iter()
        .filter(|tool| check_isa::on_path(tool).is_none())
        .collect();
    if !missing.is_empty() {
        eprintln!("the FAT boot disk needs host tools that are missing:");
        for tool in &missing {
            eprintln!("  {tool} (mtools)");
        }
        process::exit(1);
    }
}

fn require_names_fit_8_3() {
    let offenders: Vec<&str> = USER_APPS
        .into_iter()
        .filter(|name| name.len() > MAX_FAT_NAME_BYTES)
        .collect();
    if !offenders.is_empty() {
        eprintln!(
            "the FAT boot disk carries apps by name; these exceed the 8.3 stem: {offenders:?}"
        );
        process::exit(1);
    }
}

fn stamp_mbr(path: &Path) -> u32 {
    let disk_sectors = (BOOT_DISK_BYTES / SECTOR_BYTES) as u32;
    let partition_sectors = disk_sectors - PARTITION_START_LBA;
    let mut entries: [Option<Entry>; ENTRY_COUNT] = [None; ENTRY_COUNT];
    entries[0] = Some(Entry {
        boot: BOOTABLE,
        type_id: FAT16_TYPE,
        start_lba: PARTITION_START_LBA,
        sector_count: partition_sectors,
    });
    let mbr = rxpart::mbr::encode_mbr(&entries);
    let mut file = File::create(path).expect("failed to create the boot disk image");
    file.set_len(BOOT_DISK_BYTES)
        .expect("failed to size the boot disk image");
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&mbr).unwrap();
    partition_sectors
}

fn partition_index(image: &Path) -> String {
    format!(
        "-i{}@@{}",
        image.display(),
        PARTITION_START_LBA as u64 * SECTOR_BYTES
    )
}

fn mformat_volume(image: &Path, partition_sectors: u32) {
    let mut cmd = Command::new("mformat");
    cmd.arg(partition_index(image));
    cmd.args(["-h", HEADS, "-s", SECTORS_PER_TRACK, "-T", &partition_sectors.to_string()]);
    cmd.args(["-F", "-v", VOLUME_LABEL]);
    run(&mut cmd, "mformat the boot disk FAT16 volume");
}

fn mmake_boot_directory(image: &Path) {
    let mut cmd = Command::new("mmd");
    cmd.arg(partition_index(image));
    cmd.arg("::/boot");
    run(&mut cmd, "mmd the boot disk /boot directory");
}

fn mcopy_apps(image: &Path, release_dir: &Path) {
    let mut cmd = Command::new("mcopy");
    cmd.arg("-m").arg("-o").arg(partition_index(image));
    for name in USER_APPS {
        let elf = release_dir.join(name);
        if !elf.is_file() {
            eprintln!("missing user ELF {}; run `cargo xtask apps` first", elf.display());
            process::exit(1);
        }
        cmd.arg(elf);
    }
    cmd.arg("::/");
    run(&mut cmd, "mcopy the user ELFs to the boot disk root");
}

fn mcopy_boot_files(image: &Path, kernel: &Path, rom: &Path) {
    for (source, destination) in [(kernel, "::/boot/kernel.elf"), (rom, "::/boot/rom.img")] {
        if !source.is_file() {
            eprintln!("missing boot file {}", source.display());
            process::exit(1);
        }
        let mut cmd = Command::new("mcopy");
        cmd.arg("-m")
            .arg("-o")
            .arg(partition_index(image))
            .arg(source)
            .arg(destination);
        run(&mut cmd, &format!("mcopy {} to boot disk {destination}", source.display()));
    }
}
