use std::path::{Path, PathBuf};
use std::process::Command;

use rxpart::mbr::{Entry, ENTRY_COUNT, parse_mbr};
use x86_runner::chain;

use crate::arch::Arch;

const SECTOR_BYTES: usize = 512;
const BOOT_DISK_BYTES: usize = 16 * 1024 * 1024;
const BOOTABLE: u8 = 0x80;
const FAT16_TYPE: u8 = 0x06;

pub const CHAIN_KERNEL_VOLUME_NAME: &str = "KERNEL.ELF";
pub const CHAIN_INITRD_VOLUME_NAME: &str = "INITRD.BIN";

pub struct FatDisk {
    image: PathBuf,
    target: String,
}

impl FatDisk {
    pub fn path(&self) -> &Path {
        &self.image
    }

    pub fn make_directory(&self, directory: &str) {
        host_run("mmd", &["-i", &self.target, directory]);
    }

    pub fn copy(&self, source: &Path, destination: &str) {
        host_run(
            "mcopy",
            &[
                "-m",
                "-i",
                &self.target,
                &source.to_string_lossy(),
                destination,
            ],
        );
    }

    pub fn write_seed(&self, payload: &[u8], destination: &str) {
        let seed = self.image.with_extension("seed");
        std::fs::write(&seed, payload).expect("write the seed fixture");
        self.copy(&seed, destination);
        std::fs::remove_file(&seed).expect("drop the seed fixture");
    }

    pub fn install_initrd(&self) {
        let rom = user_rom_path();
        if !rom.is_file() {
            panic!(
                "missing ROM image {}; run `cargo xtask build {}` first",
                rom.display(),
                Arch::current().name()
            );
        }
        self.copy(&rom, &format!("::{CHAIN_INITRD_VOLUME_NAME}"));
    }
}

pub fn tools_available() -> bool {
    Command::new("mformat")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn volume_start_lba(image: &Path) -> u32 {
    let head = std::fs::read(image)
        .unwrap_or_else(|err| panic!("cannot read the volume header of {}: {err}", image.display()));
    assert!(head.len() >= SECTOR_BYTES, "{} is shorter than an MBR", image.display());
    let mut sector = [0u8; SECTOR_BYTES];
    sector.copy_from_slice(&head[..SECTOR_BYTES]);
    let table = parse_mbr(&sector, (head.len() / SECTOR_BYTES) as u64);
    match table.entries[0] {
        Some(entry) => entry.start_lba,
        None => panic!(
            "{} carries no slice in MBR slot 0 (table {:?})",
            image.display(),
            table.state
        ),
    }
}

pub fn read_file_from_image(image: &Path, name: &str) -> Vec<u8> {
    let target = format!("{}@@{}", image.display(), volume_start_lba(image) as usize * SECTOR_BYTES);
    let output = Command::new("mtype")
        .arg("-i")
        .arg(&target)
        .arg(format!("::{name}"))
        .output()
        .expect("mtype unavailable");
    assert!(
        output.status.success(),
        "mtype ::{name} on {} failed: {}",
        image.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

pub fn create_bootable(dir: &Path, name: &str, image_bytes: usize) -> FatDisk {
    let image = dir.join(name);
    let partition_sectors = (image_bytes / SECTOR_BYTES) as u32 - chain::PARTITION_START_LBA;
    let mut entries: [Option<Entry>; ENTRY_COUNT] = [None; ENTRY_COUNT];
    entries[0] = Some(Entry {
        boot: BOOTABLE,
        type_id: FAT16_TYPE,
        start_lba: chain::PARTITION_START_LBA,
        sector_count: partition_sectors,
    });
    let (stage1, stage2) = chain_bins();
    chain::create_sparse(&image, image_bytes as u64);
    chain::write_at(&image, 0, &chain::boot_mbr(&stage1, &entries));
    chain::write_at(
        &image,
        u64::from(chain::STAGE2_IMAGE_LBA) * SECTOR_BYTES as u64,
        &chain::stage2_blob(&stage2),
    );
    chain::mformat_volume(&image, chain::PARTITION_START_LBA, partition_sectors);
    FatDisk {
        target: mtools_target(&image, chain::PARTITION_START_LBA),
        image,
    }
}

pub fn stamp_bootable(image: &Path, kernel_elf: &Path) {
    let disk_sectors = std::fs::metadata(image)
        .unwrap_or_else(|err| panic!("cannot size the image to stamp {}: {err}", image.display()))
        .len()
        / SECTOR_BYTES as u64;
    let head = std::fs::read(image)
        .unwrap_or_else(|err| panic!("cannot read the image to stamp {}: {err}", image.display()));
    assert!(head.len() >= SECTOR_BYTES, "the image to stamp is shorter than an MBR");
    let mut sector = [0u8; SECTOR_BYTES];
    sector.copy_from_slice(&head[..SECTOR_BYTES]);
    let table = parse_mbr(&sector, disk_sectors);
    let mut entries = table.entries;
    let Some(first) = entries[0] else {
        panic!(
            "the boot master {} carries no slice in MBR slot 0 (table {:?})",
            image.display(),
            table.state
        );
    };
    assert_eq!(
        first.start_lba, chain::PARTITION_START_LBA,
        "the boot master {} slices at LBA {}, but the chain wants LBA {}",
        image.display(),
        first.start_lba,
        chain::PARTITION_START_LBA
    );
    entries[0] = Some(Entry { boot: BOOTABLE, ..first });
    let (stage1, stage2) = chain_bins();
    chain::write_at(image, 0, &chain::boot_mbr(&stage1, &entries));
    chain::write_at(
        image,
        u64::from(chain::STAGE2_IMAGE_LBA) * SECTOR_BYTES as u64,
        &chain::stage2_blob(&stage2),
    );
    chain::mcopy_file(
        image,
        chain::PARTITION_START_LBA,
        kernel_elf,
        &format!("::{CHAIN_KERNEL_VOLUME_NAME}"),
    );
}

pub fn create_boot_master(dir: &Path, kernel_elf: &Path) -> FatDisk {
    let disk = create_bootable(dir, "boot-disk.img", BOOT_DISK_BYTES);
    copy_built_els(&disk);
    disk.install_initrd();
    chain::mcopy_file(
        disk.path(),
        chain::PARTITION_START_LBA,
        kernel_elf,
        &format!("::{CHAIN_KERNEL_VOLUME_NAME}"),
    );
    disk
}

fn chain_bins() -> (Vec<u8>, Vec<u8>) {
    let (stage_dir, stage2_name) = match Arch::current().boot() {
        x86_runner::Arch::X86_32 => ("boot32", "stage2-32.bin"),
        x86_runner::Arch::X86_64 => ("boot64", "stage2-64.bin"),
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(stage_dir);
    let read = |name: &str| {
        let path = dir.join(name);
        std::fs::read(&path).unwrap_or_else(|err| {
            panic!(
                "missing staged boot-chain {} ({err}); run `cargo xtask build {}` \
                 or `cargo xtask boot-artifacts {}` to stage it",
                path.display(),
                Arch::current().name(),
                Arch::current().name(),
            )
        })
    };
    (read("stage1.bin"), read(stage2_name))
}

fn copy_built_els(disk: &FatDisk) {
    let mut args: Vec<String> = vec![
        "-m".to_string(),
        "-o".to_string(),
        "-i".to_string(),
        disk.target.clone(),
    ];
    args.extend(built_els().into_iter().map(|(_, elf)| elf.display().to_string()));
    args.push("::/".to_string());
    let arguments: Vec<&str> = args.iter().map(String::as_str).collect();
    host_run("mcopy", &arguments);
}

fn mtools_target(image: &Path, partition_start_lba: u32) -> String {
    format!(
        "{}@@{}",
        image.display(),
        partition_start_lba as usize * SECTOR_BYTES
    )
}

pub fn built_elf(app: &str) -> PathBuf {
    let elf = user_release_dir().join(app);
    if !elf.is_file() {
        panic!(
            "missing user ELF {}; run `cargo xtask build {}` first",
            elf.display(),
            Arch::current().name()
        );
    }
    elf
}

fn user_release_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(Arch::current().user_dir_name())
        .join("release")
}

fn user_rom_path() -> PathBuf {
    user_release_dir().with_file_name("rom.img")
}

fn built_els() -> Vec<(String, PathBuf)> {
    let release = user_release_dir();
    let entries = std::fs::read_dir(&release)
        .unwrap_or_else(|err| panic!("cannot read the user release directory: {err}"));
    let mut elfs: Vec<(String, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path()))
        .filter(|(name, path)| path.is_file() && !name.contains('.'))
        .collect();
    elfs.sort();
    if elfs.is_empty() {
        panic!(
            "no user ELFs in {}; run `cargo xtask build {}` first",
            release.display(),
            Arch::current().name()
        );
    }
    elfs
}

fn host_run(program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|error| panic!("{program} unavailable: {error}"));
    assert!(status.success(), "{program} {args:?} failed");
}
