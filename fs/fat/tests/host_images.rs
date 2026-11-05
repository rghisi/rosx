use fs_core::engine::{Engine, Kind, MetadataView};
use fs_core::error::{FsError, IoError};
use rxfat::device::BlockDevice;
use rxfat::FileSystem;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct ImageDevice {
    bytes: Vec<u8>,
}

impl BlockDevice for ImageDevice {
    fn block_count(&self) -> u64 {
        (self.bytes.len() / 512) as u64
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
}

struct Workspace(PathBuf);

impl Workspace {
    fn new(tag: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("rxfat-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("fixture workspace");
        Workspace(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn write_file(&self, name: &str, bytes: &[u8]) {
        fs::write(self.path(name), bytes).expect("fixture file");
    }

    fn blank_image(&self, name: &str, sectors: u64) -> PathBuf {
        let image = self.path(name);
        let file = fs::File::create(&image).expect("blank image");
        file.set_len(sectors * 512).expect("image size");
        image
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
    let status = Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|error| panic!("{program} unavailable: {error}"));
    assert!(status.success(), "{program} {args:?} failed");
}

fn pattern(seed: u8, length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| (index as u64 * u64::from(seed) + 7) as u8)
        .collect()
}

fn populate(workspace: &Workspace, image: &Path) {
    let img = image.to_str().expect("utf8 image path");
    run("mmd", &["-i", img, "::/SUB"]);
    for name in ["ALPHA.TXT", "BETA.BIN", "C.EMPTY", "LONGNAME.TXT"] {
        let source = workspace.path(name);
        run(
            "mcopy",
            &["-m", "-i", img, source.to_str().expect("utf8"), "::"],
        );
    }
    let gamma = workspace.path("SUB/GAMMA.DAT");
    run(
        "mcopy",
        &["-m", "-i", img, gamma.to_str().expect("utf8"), "::/SUB/"],
    );
}

fn write_fixtures(workspace: &Workspace) {
    workspace.write_file("ALPHA.TXT", &pattern(31, 1_000));
    workspace.write_file("BETA.BIN", &pattern(17, 3_000));
    workspace.write_file("C.EMPTY", b"");
    workspace.write_file("LONGNAME.TXT", &pattern(5, 40));
    fs::create_dir(workspace.path("SUB")).expect("sub fixture dir");
    fs::write(workspace.path("SUB/GAMMA.DAT"), pattern(13, 222)).expect("gamma fixture");
}

fn engine_over(image: &Path) -> FileSystem<ImageDevice> {
    let device = ImageDevice {
        bytes: fs::read(image).expect("read image"),
    };
    FileSystem::new(device).expect("mtools image opens")
}

fn assert_matches_fixtures(image: &Path) {
    let mut fs = engine_over(image);

    let mut listing: Vec<(String, Kind)> = Engine::readdir(&mut fs, b"/")
        .expect("root listing")
        .into_iter()
        .map(|item| (String::from_utf8(item.name).expect("utf8 name"), item.kind))
        .collect();
    listing.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        listing,
        vec![
            (String::from("alpha.txt"), Kind::Regular),
            (String::from("beta.bin"), Kind::Regular),
            (String::from("c~1.emp"), Kind::Regular),
            (String::from("longname.txt"), Kind::Regular),
            (String::from("sub"), Kind::Directory),
        ]
    );

    let sub = Engine::readdir(&mut fs, b"/SUB").expect("sub listing");
    assert_eq!(sub.len(), 1);
    assert_eq!(sub[0].name, b"gamma.dat".to_vec());
    assert_eq!(sub[0].kind, Kind::Regular);

    for (path, seed, size) in [
        ("/alpha.txt", 31u8, 1_000usize),
        ("/BETA.BIN", 17, 3_000),
        ("/sub/gamma.dat", 13, 222),
        ("/LongName.txt", 5, 40),
    ] {
        let handle = Engine::open(&mut fs, path.as_bytes())
            .unwrap_or_else(|error| panic!("open {path}: {error:?}"));
        let meta = Engine::stat(&mut fs, handle).expect("stat");
        assert_eq!(meta.size(), size as u64, "{path}");
        let mut buf = vec![0u8; size];
        let read = Engine::read(&mut fs, handle, 0, &mut buf).expect("read");
        assert_eq!(read, size, "{path}");
        assert_eq!(buf, pattern(seed, size), "{path}");
        Engine::close(&mut fs, handle).expect("close");
    }

    let empty = Engine::open(&mut fs, b"/c~1.emp").expect("open empty");
    assert_eq!(Engine::stat(&mut fs, empty).expect("stat").size(), 0);
    assert_eq!(Engine::read(&mut fs, empty, 0, &mut [0u8; 4]).expect("read"), 0);

    let mid = Engine::open(&mut fs, b"/beta.bin").expect("open beta");
    let mut tail = vec![0u8; 600];
    let read = Engine::read(&mut fs, mid, 2_700, &mut tail).expect("straddling tail");
    assert_eq!(read, 300);
    assert_eq!(tail[..300], pattern(17, 3_000)[2_700..]);
}

#[test]
fn engine_reads_a_real_mformat_fat16_image() {
    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping FAT16 differential test");
        return;
    }
    let workspace = Workspace::new("diff16");
    write_fixtures(&workspace);
    let image = workspace.blank_image("fat16.img", 10_080);
    let img = image.to_str().expect("utf8");
    run(
        "mformat",
        &["-i", img, "-h", "16", "-s", "63", "-T", "10080", "-r", "224", "-c", "1", "-v", "RXFAT16"],
    );
    populate(&workspace, &image);
    assert_matches_fixtures(&image);
}

#[test]
fn engine_reads_a_real_mformat_fat32_image() {
    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping FAT32 differential test");
        return;
    }
    let workspace = Workspace::new("diff32");
    write_fixtures(&workspace);
    let image = workspace.blank_image("fat32.img", 1_100_000);
    let img = image.to_str().expect("utf8");
    run(
        "mformat",
        &["-i", img, "-h", "16", "-s", "63", "-T", "1100000", "-F", "-v", "RXFAT32"],
    );
    populate(&workspace, &image);
    assert_matches_fixtures(&image);
}

#[test]
fn corrupting_real_image_bytes_changes_the_verdict() {
    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping corruption battery");
        return;
    }
    let workspace = Workspace::new("corrupt");
    write_fixtures(&workspace);
    let image = workspace.blank_image("fat16.img", 10_080);
    let img = image.to_str().expect("utf8");
    run(
        "mformat",
        &["-i", img, "-h", "16", "-s", "63", "-T", "10080", "-r", "224", "-c", "1", "-v", "RXFAT16"],
    );
    populate(&workspace, &image);
    let healthy = fs::read(&image).expect("read image");

    let mut no_boot_signature = healthy.clone();
    no_boot_signature[0x1FE] = 0x00;
    assert!(matches!(
        FileSystem::new(ImageDevice {
            bytes: no_boot_signature
        }),
        Err(FsError::Unsupported)
    ));

    assert!(matches!(
        FileSystem::new(ImageDevice {
            bytes: vec![0u8; healthy.len()]
        }),
        Err(FsError::Unsupported)
    ));

    let mut claims_more_than_the_device = healthy.clone();
    claims_more_than_the_device[0x13..0x15].copy_from_slice(&0xFFFFu16.to_le_bytes());
    assert!(matches!(
        FileSystem::new(ImageDevice {
            bytes: claims_more_than_the_device
        }),
        Err(FsError::DeviceTooSmall)
    ));

    let mut every_chain_ends_at_the_first_cluster = healthy.clone();
    every_chain_ends_at_the_first_cluster[512..512 + 39 * 512].fill(0xFF);
    let mut engine = FileSystem::new(ImageDevice {
        bytes: every_chain_ends_at_the_first_cluster,
    })
    .expect("geometry is intact, only the FAT is poisoned");
    assert!(Engine::readdir(&mut engine, b"/").is_ok());
    let handle = Engine::open(&mut engine, b"/beta.bin").expect("open");
    assert_eq!(
        Engine::read(&mut engine, handle, 0, &mut vec![0u8; 3_000]),
        Err(FsError::Corrupt)
    );
}
