use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use rxpart::mbr::{encode_mbr, Entry, ENTRY_COUNT};

use crate::arch::Arch;

const SECTOR_BYTES: usize = 512;
const PARTITION_START_LBA: u32 = 63;
const BOOT_DISK_BYTES: usize = 16 * 1024 * 1024;
const BOOTABLE: u8 = 0x80;
const FAT16_TYPE: u8 = 0x06;
const VOLUME_LABEL: &str = "ROSXBOOT";

pub struct FatDisk {
    image: PathBuf,
    target: String,
}

impl FatDisk {
    pub fn create(dir: &Path, name: &str, image_bytes: usize) -> FatDisk {
        let image = dir.join(name);
        let partition_sectors = (image_bytes / SECTOR_BYTES) as u32 - PARTITION_START_LBA;
        let mut entries: [Option<Entry>; ENTRY_COUNT] = [None; ENTRY_COUNT];
        entries[0] = Some(Entry {
            boot: BOOTABLE,
            type_id: FAT16_TYPE,
            start_lba: PARTITION_START_LBA,
            sector_count: partition_sectors,
        });
        let mbr = encode_mbr(&entries);
        let mut file = std::fs::File::create(&image).expect("create the FAT disk image");
        file.write_all(&mbr).expect("stamp the FAT disk MBR");
        file.set_len(image_bytes as u64)
            .expect("size the FAT disk image");
        let target = format!(
            "{}@@{}",
            image.display(),
            PARTITION_START_LBA as usize * SECTOR_BYTES
        );
        host_run(
            "mformat",
            &[
                "-i",
                &target,
                "-r",
                "224",
                "-c",
                "1",
                "-T",
                &partition_sectors.to_string(),
                "-v",
                VOLUME_LABEL,
            ],
        );
        FatDisk { image, target }
    }

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
}

pub fn tools_available() -> bool {
    Command::new("mformat")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

pub fn create_boot_disk(dir: &Path) -> FatDisk {
    let disk = FatDisk::create(dir, "boot-disk.img", BOOT_DISK_BYTES);
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
    disk
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
