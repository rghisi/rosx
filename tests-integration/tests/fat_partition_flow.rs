use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tests_integration::qemu::QemuSession;

const BOOT_BUDGET: Duration = Duration::from_secs(90);
const OP_BUDGET: Duration = Duration::from_secs(60);
const HDD_BYTES: usize = 50 * 1024 * 1024;
const MBR_SIGNATURE_OFFSET: usize = 510;
const MBR_PARTITION_TABLE_OFFSET: usize = 446;
const MBR_ENTRY_BYTES: usize = 16;
const FAT_SLICE_START_LBA: u32 = 2048;
const FAT_SLICE_SECTORS: u32 = 20_160;
const EMPTY_SLICE_START_LBA: u32 = FAT_SLICE_START_LBA + FAT_SLICE_SECTORS;
const EMPTY_SLICE_SECTORS: u32 = 4_096;
const GREETING: &[u8] = b"hello from the host mtools\n";

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

fn host_run(program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|error| panic!("{program} unavailable: {error}"));
    assert!(status.success(), "{program} {args:?} failed");
}

fn host_run_capture(program: &str, args: &[&str]) -> Vec<u8> {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("{program} unavailable: {error}"));
    assert!(output.status.success(), "{program} {args:?} failed");
    output.stdout
}

fn stamp_mbr_entry(image: &mut [u8], slot: usize, type_id: u8, start_lba: u32, sector_count: u32) {
    let entry =
        &mut image[MBR_PARTITION_TABLE_OFFSET + slot * MBR_ENTRY_BYTES..][..MBR_ENTRY_BYTES];
    entry[0] = if slot == 0 { 0x80 } else { 0x00 };
    entry[4] = type_id;
    entry[8..12].copy_from_slice(&start_lba.to_le_bytes());
    entry[12..16].copy_from_slice(&sector_count.to_le_bytes());
}

fn fat_offset_argument(disk: &Path) -> String {
    let image = disk.to_str().expect("utf8 image path");
    format!("{image}@@{}", FAT_SLICE_START_LBA as usize * 512)
}

fn build_partitioned_fat_hdd(disk: &Path) -> String {
    std::fs::write(disk, vec![0u8; HDD_BYTES]).expect("create the hdd image");
    {
        let mut image = std::fs::read(disk).expect("read the hdd image");
        stamp_mbr_entry(&mut image, 0, 0x06, FAT_SLICE_START_LBA, FAT_SLICE_SECTORS);
        stamp_mbr_entry(&mut image, 1, 0x82, EMPTY_SLICE_START_LBA, EMPTY_SLICE_SECTORS);
        image[MBR_SIGNATURE_OFFSET] = 0x55;
        image[MBR_SIGNATURE_OFFSET + 1] = 0xAA;
        std::fs::write(disk, image).expect("stamp the mbr");
    }
    let target = fat_offset_argument(disk);
    host_run(
        "mformat",
        &[
            "-i",
            &target,
            "-h",
            "16",
            "-s",
            "63",
            "-T",
            &FAT_SLICE_SECTORS.to_string(),
            "-r",
            "224",
            "-c",
            "1",
            "-v",
            "ROSFAT16",
        ],
    );
    let seed = disk.with_extension("greeting.txt");
    std::fs::write(&seed, GREETING).expect("write the greeting fixture");
    host_run(
        "mcopy",
        &[
            "-m",
            "-i",
            &target,
            seed.to_str().expect("utf8"),
            "::greeting.txt",
        ],
    );
    std::fs::remove_file(&seed).expect("drop the greeting fixture");
    target
}

#[test]
fn fat_partition_mounts_by_autodetection_and_survives_reboot() {
    let volume_dir = tempfile::tempdir().expect("tempdir for the fat hdd image");
    let disk = volume_dir.path().join("hdd-fat.img");
    let fat_target = build_partitioned_fat_hdd(&disk);

    {
        let mut session = QemuSession::spawn_with_hdd_disk(&disk);
        session.expect_output("[ATAD] ata0 drive1 ready:", BOOT_BUDGET);
        session.expect_output("[BLOCKD] ready", BOOT_BUDGET);
        session.expect_output("rose>", BOOT_BUDGET);

        let listing = run(&mut session, "attach ATA", "attach registered ATA");
        assert!(
            listing.contains("ata0d1p0  ") && listing.contains("20160x512"),
            "the fat slice must be curated into the namespace: {listing}"
        );
        assert!(
            listing.contains("ata0d1p1  ") && listing.contains("4096x512"),
            "the empty slice must be curated too: {listing}"
        );
        assert!(
            !listing.contains("ata0d1  "),
            "the table wins: the whole drive must not exist: {listing}"
        );

        run(&mut session, "mkdir /fat", "mkdir ok /fat");
        let reply = run(&mut session, "mount ata0d1p1 /fat", "mount failed: Vfs(NotFormatted)");
        assert!(
            reply.contains("NotFormatted"),
            "a zeroed slice must be refused honestly: {reply}"
        );

        run(&mut session, "mount ata0d1p0 /fat", "mount ok device=ata0d1p0 dir=/fat");

        let listing = run(&mut session, "ls /fat", "rose>");
        assert!(
            listing.contains("greeting.txt"),
            "the mtools-seeded file must list: {listing}"
        );

        run(&mut session, "mkdir /fat/sub", "mkdir ok /fat/sub");
        run(&mut session, "cp /pi /fat/sub/pi", "cp ok /pi -> /fat/sub/pi");
        run(&mut session, "cp /fat/greeting.txt /readback", "cp ok /fat/greeting.txt -> /readback");

        let listing = run(&mut session, "ls /fat/sub", "rose>");
        assert!(listing.contains("pi"), "the copy must list on fat: {listing}");
    }

    let mut session = QemuSession::spawn_with_hdd_disk(&disk);
    session.expect_output("rose>", BOOT_BUDGET);

    run(&mut session, "attach ATA", "attach registered ATA");
    run(&mut session, "mkdir /fat", "mkdir ok /fat");
    run(&mut session, "mount ata0d1p0 /fat", "mount ok device=ata0d1p0 dir=/fat");

    let listing = run(&mut session, "ls /fat", "rose>");
    assert!(
        listing.contains("greeting.txt") && listing.contains("sub"),
        "both the seeded and the written entries must survive the reboot: {listing}"
    );

    let listing = run(&mut session, "ls /fat/sub", "rose>");
    assert!(listing.contains("pi"), "the written file must survive: {listing}");

    let on_host = host_run_capture("mtype", &[
        "-i",
        &fat_target,
        "::greeting.txt",
    ]);
    assert_eq!(
        on_host, GREETING,
        "the guest must never corrupt the seeded mtools content"
    );
    let host_listing = host_run_capture("mdir", &["-i", &fat_target, "::/sub"]);
    let host_listing = String::from_utf8_lossy(&host_listing).to_lowercase();
    assert!(
        host_listing.contains("pi"),
        "the file written by the guest must be visible to mtools: {host_listing}"
    );
}
