use fat_image::{FatDisk, FatKind};
use fs_core::engine::Engine;
use fs_core::error::IoError;
use rxfat::device::BlockDevice;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct SectorDevice<'a> {
    bytes: &'a mut Vec<u8>,
}

impl BlockDevice for SectorDevice<'_> {
    fn block_count(&self) -> u64 {
        self.bytes.len() as u64 / 512
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let at = block as usize * 512;
        let Some(region) = self.bytes.get(at..at + 512) else {
            return Err(IoError::Device);
        };
        if buf.len() != 512 {
            return Err(IoError::Device);
        }
        buf.copy_from_slice(region);
        Ok(())
    }

    fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError> {
        let at = block as usize * 512;
        if buf.len() != 512 {
            return Err(IoError::Device);
        }
        let Some(region) = self.bytes.get_mut(at..at + 512) else {
            return Err(IoError::Device);
        };
        region.copy_from_slice(buf);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), IoError> {
        Ok(())
    }
}

struct Workspace(PathBuf);

impl Workspace {
    fn new(tag: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("fat-image-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("differential workspace");
        Workspace(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mtools_missing() -> bool {
    Command::new("mformat")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
}

fn run(program: &str, args: &[&str]) {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("{program} unavailable: {error}"));
    assert!(
        output.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index * 13 + 5) as u8).collect()
}

fn extract(workspace: &Workspace, image: &Path, entry: &str) -> Vec<u8> {
    let out = workspace.path("extracted");
    run(
        "mcopy",
        &[
            "-i",
            image.to_str().expect("utf8 image path"),
            &format!("::{entry}"),
            out.to_str().expect("utf8 out path"),
        ],
    );
    fs::read(&out).expect("extracted file")
}

#[test]
fn mtools_reads_the_fat16_writer_output() {
    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping FAT16 writer differential test");
        return;
    }
    let workspace = Workspace::new("fat16-writer");
    let image = workspace.path("fat16.img");
    let mut disk = FatDisk::new(FatKind::Fat16, 8 * 1024 * 1024).unwrap();
    let big = pattern(5_000);
    disk.add_file("PI", &big).unwrap();
    disk.add_file("NOTES.TXT", &pattern(600)).unwrap();
    disk.add_file("EMPTY", &[]).unwrap();
    disk.write_to(&image).unwrap();

    let img = image.to_str().expect("utf8 image path");
    run("mdir", &["-i", img, "::PI"]);
    run("mdir", &["-i", img, "::NOTES.TXT"]);
    run("mdir", &["-i", img, "::EMPTY"]);
    assert_eq!(extract(&workspace, &image, "PI"), big);
    assert_eq!(extract(&workspace, &image, "NOTES.TXT"), pattern(600));
    assert!(extract(&workspace, &image, "EMPTY").is_empty());
}

#[test]
fn mtools_reads_the_fat32_writer_output_with_a_grown_root() {
    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping FAT32 writer differential test");
        return;
    }
    let workspace = Workspace::new("fat32-writer");
    let image = workspace.path("fat32.img");
    let mut disk = FatDisk::new(FatKind::Fat32, 64 * 1024 * 1024).unwrap();
    for index in 0..20u32 {
        disk.add_file(&format!("FILE{index:03}"), &pattern(600))
            .unwrap();
    }
    disk.write_to(&image).unwrap();

    let img = image.to_str().expect("utf8 image path");
    run("mdir", &["-i", img, "::FILE017"]);
    assert_eq!(extract(&workspace, &image, "FILE017"), pattern(600));
    assert_eq!(extract(&workspace, &image, "FILE000"), pattern(600));
}

fn grow_with_rxfat(image: &Path) {
    let mut bytes = fs::read(image).expect("read image");
    {
        let mut volume = rxfat::FileSystem::new(SectorDevice {
            bytes: &mut bytes,
        })
        .expect("rxfat opens its own kind of volume");
        let grown = Engine::create(&mut volume, b"/grown.dat").expect("create");
        Engine::write(&mut volume, grown, 0, &pattern(1_500)).expect("write");
        Engine::close(&mut volume, grown).expect("close");
        Engine::mkdir(&mut volume, b"/work").expect("mkdir");
        let inside = Engine::create(&mut volume, b"/work/inside").expect("nested create");
        Engine::write(&mut volume, inside, 0, b"deep").expect("nested write");
        Engine::close(&mut volume, inside).expect("close");
        assert_eq!(Engine::remove(&mut volume, b"/PI"), Ok(()));
    }
    fs::write(image, &bytes).expect("flush grown image");
}

fn assert_grown_volume_reads_back(workspace: &Workspace, image: &Path) {
    let img = image.to_str().expect("utf8 image path");
    run("mdir", &["-i", img, "::GROWN.DAT"]);
    run("mdir", &["-i", img, "::WORK"]);
    run("mdir", &["-i", img, "::WORK/INSIDE"]);
    assert_eq!(extract(workspace, image, "GROWN.DAT"), pattern(1_500));
    assert_eq!(extract(workspace, image, "WORK/INSIDE"), b"deep");
    let deleted = Command::new("mdir")
        .args(["-i", img, "::PI"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("mdir");
    assert!(!deleted.success(), "the deleted PI is still visible to mtools");
}

#[test]
fn mtools_reads_a_fat16_volume_grown_by_rxfat() {
    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping FAT16 grown-by-rxfat test");
        return;
    }
    let workspace = Workspace::new("fat16-grown");
    let image = workspace.path("fat16.img");
    let mut disk = FatDisk::new(FatKind::Fat16, 8 * 1024 * 1024).unwrap();
    disk.add_file("PI", &pattern(5_000)).unwrap();
    disk.add_file("NOTES.TXT", &pattern(600)).unwrap();
    disk.write_to(&image).unwrap();
    grow_with_rxfat(&image);
    assert_grown_volume_reads_back(&workspace, &image);
}

#[test]
fn mtools_reads_a_fat32_volume_grown_by_rxfat() {
    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping FAT32 grown-by-rxfat test");
        return;
    }
    let workspace = Workspace::new("fat32-grown");
    let image = workspace.path("fat32.img");
    let mut disk = FatDisk::new(FatKind::Fat32, 64 * 1024 * 1024).unwrap();
    disk.add_file("PI", &pattern(5_000)).unwrap();
    for index in 0..20u32 {
        disk.add_file(&format!("FILE{index:03}"), &pattern(600))
            .unwrap();
    }
    disk.write_to(&image).unwrap();
    grow_with_rxfat(&image);
    assert_grown_volume_reads_back(&workspace, &image);
}
