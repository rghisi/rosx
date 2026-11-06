use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const OP_BUDGET: Duration = Duration::from_secs(60);
const HDD_BYTES: usize = 50 * 1024 * 1024;

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
fn init_prefers_the_disk_shell_and_falls_back_to_the_rom_one() {
    {
        let mut diskless = QemuSession::spawn();
        let start = diskless.mark();
        diskless.expect_output("[INIT] init.lst: 7 services", BOOT_BUDGET);
        diskless.expect_output("[INIT] fallback shell", BOOT_BUDGET);
        diskless.expect_output("rose>", BOOT_BUDGET);
        let boot = diskless.content_since(start);
        assert!(
            !boot.contains("[INIT] handoff"),
            "a machine with no disk must not claim a handoff: {boot}"
        );
        run(&mut diskless, "ls", "rose>");
    }

    let volume_dir = tempfile::tempdir().expect("tempdir for the handoff disk");
    let disk = volume_dir.path().join("hdd-50mb.img");
    std::fs::write(&disk, vec![0u8; HDD_BYTES]).expect("create the hdd image");

    {
        let mut session = QemuSession::spawn_with_hdd_disk(&disk);
        session.expect_output("[VFSD] automount hdd0: mount skipped: NotFormatted", BOOT_BUDGET);
        session.expect_output("[INIT] fallback shell", BOOT_BUDGET);
        session.expect_output("rose>", BOOT_BUDGET);

        run(&mut session, "format hdd0", "format ok device=hdd0");
        run(&mut session, "mount hdd0 /hdd", "mount ok device=hdd0 dir=/hdd");
        run(&mut session, "cp shell hdd/shell", "cp ok shell -> hdd/shell");
    }

    {
        let mut provisioned = QemuSession::spawn_with_hdd_disk(&disk);
        let start = provisioned.mark();
        provisioned.expect_output("[VFSD] automounted hdd0 at /hdd", BOOT_BUDGET);
        provisioned.expect_output("[INIT] handoff hdd/shell", BOOT_BUDGET);
        provisioned.expect_output("rose>", BOOT_BUDGET);
        let boot = provisioned.content_since(start);
        assert!(
            !boot.contains("[INIT] fallback"),
            "a disk shell must be handed off, not fallen back from: {boot}"
        );

        let listing = run(&mut provisioned, "ls /hdd", "rose>");
        assert!(
            listing.contains("shell"),
            "the handed-off shell must live on the disk: {listing}"
        );
    }
}
