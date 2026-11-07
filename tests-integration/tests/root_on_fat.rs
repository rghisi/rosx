use fat_image::{FatDisk, FatKind};
use std::time::Duration;
use tests_integration::arch::not_ported_to_x86_64;
use tests_integration::kernel_build::user_elf;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const OP_BUDGET: Duration = Duration::from_secs(60);
const FAT_BYTES: u64 = 50 * 1024 * 1024;

fn run(session: &mut QemuSession, command: &str, needle: &str) -> String {
    let mark = session.mark();
    session.send_line(command);
    session.expect_output_since(mark, needle, OP_BUDGET);
    if needle != "rose>" {
        let after = session.mark();
        session.expect_output_since(after, "rose>", OP_BUDGET);
    }
    session.content_since(mark)
}

#[test]
fn the_fat_volume_on_drive0_boots_as_the_root_volume() {
    if not_ported_to_x86_64() {
        return;
    }

    {
        let mut blank = QemuSession::spawn();
        let start = blank.mark();
        blank.expect_output("[VFSD] root root0: no FAT volume, rom fallback", BOOT_BUDGET);
        blank.expect_output("[INIT] fallback shell", BOOT_BUDGET);
        blank.expect_output("rose>", BOOT_BUDGET);
        let listing = run(&mut blank, "ls", "rose>");
        assert!(
            listing.contains("tetris"),
            "a blank drive0 must keep today's rom root: {listing}"
        );
    }

    let image_dir = tempfile::tempdir().expect("tempdir for the root disk");
    let disk = image_dir.path().join("root-fat.img");
    {
        let mut volume = FatDisk::new(FatKind::Fat16, FAT_BYTES).expect("fat16 volume");
        volume
            .add_file("MARKER.BIN", b"the root comes from the disk\n")
            .expect("marker fits");
        for app in ["mem", "mkdir", "format"] {
            let elf = std::fs::read(user_elf(app)).expect("read the app ELF");
            volume.add_file(app, &elf).expect("app fits");
        }
        volume.write_to(&disk).expect("write the root image");
    }

    {
        let mut session = QemuSession::spawn_with_drive0(&disk);
        session.expect_output("[VFSD] root root0 adopted from ATA 0", BOOT_BUDGET);
        session.expect_output("[INIT] fallback shell", BOOT_BUDGET);
        session.expect_output("rose>", BOOT_BUDGET);

        let listing = run(&mut session, "ls", "rose>");
        assert!(
            listing.contains("marker.bin"),
            "the marker lives on the fat root: {listing}"
        );
        assert!(
            listing.contains("mem"),
            "the disk copy must answer the bare name: {listing}"
        );
        assert!(
            !listing.contains("tetris"),
            "the rom roster must stay out of a device root: {listing}"
        );

        run(&mut session, "mem", "KSTATSD memory snapshot");
        run(&mut session, "mkdir data", "mkdir failed: Vfs(Unsupported)");
        run(&mut session, "format root0", "format failed: Vfs(Busy)");
    }
}
