use fat_image::{FatDisk, FatKind};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
