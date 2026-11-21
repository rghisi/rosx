use std::path::Path;
use std::path::PathBuf;
use std::process;

use rxpart::mbr::ENTRY_COUNT;
use rxpart::mbr::Entry;
use rxpart::mbr::SECTOR_BYTES;
use rxpart::mbr::encode_mbr;
use x86_runner::Arch;
use x86_runner::chain;

use crate::arch::ArchSpec;
use crate::arch::USER_APPS;
use crate::boot_image;
use crate::check_isa::on_path;
use crate::workspace_root;

const FORWARD_LOOKING_PARTITION_START_LBA: u32 = 63;
const BOOT_DISK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FAT_NAME_BYTES: usize = 8;
pub const KERNEL_VOLUME_NAME: &str = "KERNEL.ELF";

pub fn boot_disk_path(spec: &ArchSpec) -> PathBuf {
    workspace_root()
        .join("target")
        .join(format!("{}-boot-disk.img", spec.kernel_dir_name))
}

pub fn create_boot_disk(debug: bool, spec: &ArchSpec) -> PathBuf {
    require_fat_tools();
    require_names_fit_8_3();
    let path = boot_disk_path(spec);
    match spec.boot {
        Arch::X86_64 => create_forward_looking_disk(debug, spec, &path),
        Arch::X86_32 => create_bootable_disk(debug, spec, &path),
    }
    path
}

fn require_fat_tools() {
    let missing: Vec<&str> = ["mformat", "mmd", "mcopy", "mdir"]
        .into_iter()
        .filter(|tool| on_path(tool).is_none())
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

fn create_forward_looking_disk(debug: bool, spec: &ArchSpec, path: &Path) {
    let partition_sectors = stamp_forward_looking_mbr(path);
    chain::mformat_volume(path, FORWARD_LOOKING_PARTITION_START_LBA, partition_sectors);
    fill_volume(path, FORWARD_LOOKING_PARTITION_START_LBA, debug, spec);
    println!("==> wrote FAT boot disk {}", path.display());
}

fn create_bootable_disk(debug: bool, spec: &ArchSpec, path: &Path) {
    let (stage1, stage2) = boot_image::stage_boot32_artifacts();
    let partition_sectors =
        (BOOT_DISK_BYTES / SECTOR_BYTES as u64) as u32 - chain::PARTITION_START_LBA;
    let mut entries: [Option<Entry>; ENTRY_COUNT] = [None; ENTRY_COUNT];
    entries[0] = Some(Entry {
        boot: chain::BOOTABLE,
        type_id: chain::FAT16_TYPE,
        start_lba: chain::PARTITION_START_LBA,
        sector_count: partition_sectors,
    });
    chain::create_sparse(path, BOOT_DISK_BYTES);
    chain::write_at(path, 0, &chain::boot_mbr(&stage1, &entries));
    chain::write_at(
        path,
        u64::from(chain::STAGE2_IMAGE_LBA) * SECTOR_BYTES as u64,
        &chain::stage2_blob(&stage2),
    );
    chain::mformat_volume(path, chain::PARTITION_START_LBA, partition_sectors);
    fill_volume(path, chain::PARTITION_START_LBA, debug, spec);
    let listing = chain::mdir_listing(path, chain::PARTITION_START_LBA);
    assert!(
        listing.contains("KERNEL") && listing.contains("ELF"),
        "the bootable disk volume carries no KERNEL.ELF for stage2; mdir says: {listing}"
    );
    println!("==> wrote bootable unified FAT disk {}", path.display());
}

fn stamp_forward_looking_mbr(path: &Path) -> u32 {
    let disk_sectors = (BOOT_DISK_BYTES / SECTOR_BYTES as u64) as u32;
    let partition_sectors = disk_sectors - FORWARD_LOOKING_PARTITION_START_LBA;
    let mut entries: [Option<Entry>; ENTRY_COUNT] = [None; ENTRY_COUNT];
    entries[0] = Some(Entry {
        boot: chain::BOOTABLE,
        type_id: chain::FAT16_TYPE,
        start_lba: FORWARD_LOOKING_PARTITION_START_LBA,
        sector_count: partition_sectors,
    });
    let mbr = encode_mbr(&entries);
    chain::create_sparse(path, BOOT_DISK_BYTES);
    chain::write_at(path, 0, &mbr);
    partition_sectors
}

fn fill_volume(path: &Path, partition_start_lba: u32, debug: bool, spec: &ArchSpec) {
    chain::mmd_directory(path, partition_start_lba, "::/boot");
    let release_dir = spec.user_release_dir();
    for name in USER_APPS {
        let elf = release_dir.join(name);
        if !elf.is_file() {
            eprintln!("missing user ELF {}; run `cargo xtask apps` first", elf.display());
            process::exit(1);
        }
        chain::mcopy_file(path, partition_start_lba, &elf, &format!("::/{name}"));
    }
    let kernel = spec.kernel_elf(if debug { "debug" } else { "release" });
    let kernel_destination = match spec.boot {
        Arch::X86_64 => "::/boot/kernel.elf".to_string(),
        Arch::X86_32 => format!("::/{KERNEL_VOLUME_NAME}"),
    };
    for (source, destination) in
        [(kernel, kernel_destination), (spec.rom_path(), "::/boot/rom.img".to_string())]
    {
        if !source.is_file() {
            eprintln!("missing boot file {}", source.display());
            process::exit(1);
        }
        chain::mcopy_file(path, partition_start_lba, &source, &destination);
    }
}
