use fs_core::engine::{Engine, Kind, MetadataView};
use fs_core::error::{FsError, IoError};
use rxfat::device::BlockDevice;
use rxfat::FileSystem;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

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

fn run_capture(program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("{program} unavailable: {error}"));
    assert!(output.status.success(), "{program} {args:?} failed");
    String::from_utf8(output.stdout).expect("utf8 output")
}

#[derive(Clone)]
struct SharedImage {
    bytes: Rc<RefCell<Vec<u8>>>,
}

impl BlockDevice for SharedImage {
    fn block_count(&self) -> u64 {
        self.bytes.borrow().len() as u64 / 512
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let bytes = self.bytes.borrow();
        let at = block as usize * 512;
        let Some(region) = bytes.get(at..at + 512) else {
            return Err(IoError::Device);
        };
        if buf.len() != 512 {
            return Err(IoError::Device);
        }
        buf.copy_from_slice(region);
        Ok(())
    }

    fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError> {
        let mut bytes = self.bytes.borrow_mut();
        let at = block as usize * 512;
        if buf.len() != 512 {
            return Err(IoError::Device);
        }
        let Some(region) = bytes.get_mut(at..at + 512) else {
            return Err(IoError::Device);
        };
        region.copy_from_slice(buf);
        Ok(())
    }
}

fn shared_engine(image: &Path) -> (FileSystem<SharedImage>, Rc<RefCell<Vec<u8>>>) {
    let bytes = Rc::new(RefCell::new(fs::read(image).expect("read image")));
    let engine = FileSystem::new(SharedImage {
        bytes: Rc::clone(&bytes),
    })
    .expect("mtools image opens for writing");
    (engine, bytes)
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

fn mdir(image: &Path, target: &str) -> String {
    let img = image.to_str().expect("utf8 image path");
    run_capture("mdir", &["-i", img, target])
}

fn mcopy_out(image: &Path, from: &str, to: &Path) {
    let img = image.to_str().expect("utf8 image path");
    run(
        "mcopy",
        &["-i", img, from, to.to_str().expect("utf8 host path")],
    );
}

fn mutate_volume(image: &Path) {
    let (mut fs, bytes) = shared_engine(image);
    let payload = pattern(23, 2_000);
    let written = Engine::create(&mut fs, b"/written.dat").expect("create");
    assert_eq!(
        Engine::write(&mut fs, written, 0, &payload).expect("write"),
        2_000
    );
    Engine::close(&mut fs, written).expect("close");
    Engine::mkdir(&mut fs, b"/guest").expect("mkdir");
    let inner = Engine::create(&mut fs, b"/guest/inner.txt").expect("nested create");
    Engine::write(&mut fs, inner, 0, b"nested").expect("nested write");
    Engine::close(&mut fs, inner).expect("close");
    assert_eq!(Engine::remove(&mut fs, b"/alpha.txt"), Ok(()));
    let beta = Engine::open(&mut fs, b"/beta.bin").expect("open beta");
    Engine::write(&mut fs, beta, 0, b"PATCH").expect("patch beta");
    Engine::close(&mut fs, beta).expect("close");
    fs::write(image, bytes.borrow().as_slice()).expect("flush mutated image");
}

fn mdir_lists(listing: &str, name: &str) -> bool {
    listing
        .lines()
        .any(|line| line.trim_start().starts_with(name))
}

fn assert_mutations_visible_to_mtools(workspace: &Workspace, image: &Path) {
    let listing = mdir(image, "::");
    assert!(mdir_lists(&listing, "WRITTEN"), "listing: {listing}");
    assert!(mdir_lists(&listing, "GUEST"), "listing: {listing}");
    assert!(!mdir_lists(&listing, "ALPHA"), "deleted file leaked: {listing}");
    let guest = mdir(image, "::GUEST");
    assert!(mdir_lists(&guest, "INNER"), "guest dir: {guest}");

    let written = workspace.path("out-written.dat");
    mcopy_out(image, "::WRITTEN.DAT", &written);
    assert_eq!(fs::read(&written).expect("mcopy written"), pattern(23, 2_000));
    let inner = workspace.path("out-inner.txt");
    mcopy_out(image, "::GUEST/INNER.TXT", &inner);
    assert_eq!(fs::read(&inner).expect("mcopy inner"), b"nested");
    let beta = workspace.path("out-beta.bin");
    mcopy_out(image, "::BETA.BIN", &beta);
    let beta = fs::read(&beta).expect("mcopy beta");
    assert_eq!(&beta[..5], b"PATCH");
    assert_eq!(beta.len(), 3_000);
    assert_eq!(&beta[5..], &pattern(17, 3_000)[5..]);
}

#[test]
fn files_written_by_rxfat_read_back_through_mtools_fat16() {
    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping FAT16 write differential");
        return;
    }
    let workspace = Workspace::new("write16");
    write_fixtures(&workspace);
    let image = workspace.blank_image("fat16.img", 10_080);
    let img = image.to_str().expect("utf8");
    run(
        "mformat",
        &["-i", img, "-h", "16", "-s", "63", "-T", "10080", "-r", "224", "-c", "1", "-v", "RXFAT16"],
    );
    populate(&workspace, &image);
    mutate_volume(&image);
    assert_mutations_visible_to_mtools(&workspace, &image);

    let mut remounted = engine_over(&image);
    assert_eq!(
        Engine::lookup(&mut remounted, b"/alpha.txt").err(),
        Some(FsError::NotFound)
    );
    assert_eq!(
        Engine::lookup(&mut remounted, b"/written.dat").expect("written").size(),
        2_000
    );
    assert_eq!(
        Engine::lookup(&mut remounted, b"/guest/inner.txt").expect("inner").size(),
        6
    );
}

#[test]
fn files_written_by_rxfat_read_back_through_mtools_fat32() {
    if mtools_missing() {
        eprintln!("note: mtools not installed, skipping FAT32 write differential");
        return;
    }
    let workspace = Workspace::new("write32");
    write_fixtures(&workspace);
    let image = workspace.blank_image("fat32.img", 1_100_000);
    let img = image.to_str().expect("utf8");
    run(
        "mformat",
        &["-i", img, "-h", "16", "-s", "63", "-T", "1100000", "-F", "-v", "RXFAT32"],
    );
    populate(&workspace, &image);
    mutate_volume(&image);
    assert_mutations_visible_to_mtools(&workspace, &image);
}
