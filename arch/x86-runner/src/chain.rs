use std::fs::File;
use std::fs::OpenOptions;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use rxpart::mbr::ENTRY_COUNT;
use rxpart::mbr::Entry;
use rxpart::mbr::SECTOR_BYTES;
use rxpart::mbr::TABLE_OFFSET;
use rxpart::mbr::encode_mbr;

pub const STAGE2_IMAGE_LBA: u32 = 8;
pub const STAGE2_HEADER_MAGIC: &[u8; 4] = b"ROS2";
pub const STAGE2_HEADER_BYTES: usize = SECTOR_BYTES;
pub const STAGE2_MAX_BIN_BYTES: usize = 64 * 1024 + 512;

pub const PARTITION_START_LBA: u32 = 2048;
pub const BOOTABLE: u8 = 0x80;
pub const FAT16_TYPE: u8 = 0x06;
pub const VOLUME_LABEL: &str = "ROSXBOOT";
pub const HEADS: &str = "16";
pub const SECTORS_PER_TRACK: &str = "63";
pub const SECTORS_PER_CLUSTER: &str = "2";

pub fn stage2_blob(bin: &[u8]) -> Vec<u8> {
    assert!(
        !bin.is_empty() && bin.len() <= STAGE2_MAX_BIN_BYTES,
        "stage2 bin size {}/{}",
        bin.len(),
        STAGE2_MAX_BIN_BYTES
    );
    let padded_bin = bin.len().div_ceil(SECTOR_BYTES) * SECTOR_BYTES;
    let total_bytes = (STAGE2_HEADER_BYTES + padded_bin) as u32;
    let mut blob = vec![0u8; total_bytes as usize];
    blob[0..4].copy_from_slice(STAGE2_HEADER_MAGIC);
    blob[4..8].copy_from_slice(&0u32.to_le_bytes());
    blob[8..12].copy_from_slice(&total_bytes.to_le_bytes());
    blob[12..16].copy_from_slice(&0u32.to_le_bytes());
    blob[STAGE2_HEADER_BYTES..STAGE2_HEADER_BYTES + bin.len()].copy_from_slice(bin);
    blob
}

pub fn boot_mbr(stage1: &[u8], entries: &[Option<Entry>; ENTRY_COUNT]) -> Vec<u8> {
    assert_eq!(stage1.len(), SECTOR_BYTES, "the stage1 image must be exactly one boot sector");
    assert_eq!(&stage1[SECTOR_BYTES - 2..], &[0x55, 0xAA], "the stage1 image must carry 0x55AA");
    assert!(
        stage1[TABLE_OFFSET..SECTOR_BYTES - 2].iter().all(|&byte| byte == 0),
        "the stage1 image reaches its partition table zone"
    );
    let mut mbr = encode_mbr(entries).to_vec();
    mbr[..TABLE_OFFSET].copy_from_slice(&stage1[..TABLE_OFFSET]);
    mbr
}

pub fn create_sparse(path: &Path, bytes: u64) {
    let file = File::create(path).expect("failed to create the boot image");
    file.set_len(bytes)
        .expect("failed to size the boot image sparse file");
}

pub fn write_at(path: &Path, offset: u64, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .expect("failed to open the boot image for writing");
    file.seek(SeekFrom::Start(offset))
        .expect("failed to seek the boot image");
    file.write_all(bytes)
        .expect("failed to write a region of the boot image");
}

pub fn partition_index(image: &Path, partition_start_lba: u32) -> String {
    format!(
        "-i{}@@{}",
        image.display(),
        u64::from(partition_start_lba) * SECTOR_BYTES as u64
    )
}

pub fn mformat_volume(image: &Path, partition_start_lba: u32, partition_sectors: u32) {
    let arguments: Vec<String> = std::iter::once(partition_index(image, partition_start_lba))
        .chain([
            "-h".to_string(),
            HEADS.to_string(),
            "-s".to_string(),
            SECTORS_PER_TRACK.to_string(),
            "-T".to_string(),
            partition_sectors.to_string(),
            "-c".to_string(),
            SECTORS_PER_CLUSTER.to_string(),
            "-v".to_string(),
            VOLUME_LABEL.to_string(),
        ])
        .collect();
    host_run("mformat", &arguments, "mformat the FAT16 boot volume");
}

pub fn mmd_directory(image: &Path, partition_start_lba: u32, directory: &str) {
    let arguments = [partition_index(image, partition_start_lba), directory.to_string()];
    host_run("mmd", &arguments, &format!("mmd the boot volume directory {directory}"));
}

pub fn mcopy_file(image: &Path, partition_start_lba: u32, source: &Path, destination: &str) {
    let arguments = [
        "-m".to_string(),
        "-o".to_string(),
        partition_index(image, partition_start_lba),
        source.display().to_string(),
        destination.to_string(),
    ];
    host_run(
        "mcopy",
        &arguments,
        &format!("mcopy {} onto the boot volume at {destination}", source.display()),
    );
}

pub fn mdelete_file(image: &Path, partition_start_lba: u32, path: &str) {
    let offset = u64::from(partition_start_lba) * SECTOR_BYTES as u64;
    let arguments = [
        "-i".to_string(),
        format!("{}@@{}", image.display(), offset),
        path.to_string(),
    ];
    host_run("mdel", &arguments, &format!("mdel {path} from the boot volume"));
}

pub fn mdir_listing(image: &Path, partition_start_lba: u32) -> String {
    let arguments = ["-/".to_string(), partition_index(image, partition_start_lba), "::".to_string()];
    let listing = host_output("mdir", &arguments);
    assert!(
        listing.contains(VOLUME_LABEL),
        "mdir cannot see the {VOLUME_LABEL} volume at LBA {partition_start_lba}; output: {listing}"
    );
    listing
}

fn host_run(program: &str, arguments: &[String], what: &str) {
    let mut cmd = Command::new(program);
    cmd.args(arguments);
    let status = cmd
        .status()
        .unwrap_or_else(|err| panic!("failed to run {program} to {what}: {err}"));
    assert!(status.success(), "{program} failed to {what} (status {status})");
}

fn host_output(program: &str, arguments: &[String]) -> String {
    let mut cmd = Command::new(program);
    cmd.args(arguments);
    let output = cmd
        .output()
        .unwrap_or_else(|err| panic!("failed to run {program}: {err}"));
    assert!(output.status.success(), "{program} failed (status {})", output.status);
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_field(blob: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(blob[offset..offset + 4].try_into().unwrap())
    }

    #[test]
    fn stage2_blob_encodes_the_ros2_header() {
        let bin = vec![0xFAu8; 102];
        let blob = stage2_blob(&bin);
        assert_eq!(&blob[0..4], b"ROS2");
        assert_eq!(header_field(&blob, 4), 0);
        assert_eq!(header_field(&blob, 8), 1024);
        assert_eq!(header_field(&blob, 12), 0);
        assert_eq!(blob.len(), 1024);
        assert_eq!(&blob[STAGE2_HEADER_BYTES..STAGE2_HEADER_BYTES + 102], &bin[..]);
        assert!(blob[STAGE2_HEADER_BYTES + 102..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn stage2_blob_rounds_the_image_up_to_sector_multiples() {
        let blob = stage2_blob(&[0u8; 1]);
        assert_eq!(blob.len(), STAGE2_HEADER_BYTES + SECTOR_BYTES);
        assert_eq!(header_field(&blob, 8), 1024);
    }

    #[test]
    fn stage2_blob_accepts_the_budget_boundary() {
        let blob = stage2_blob(&vec![0x90u8; STAGE2_MAX_BIN_BYTES]);
        assert_eq!(blob.len(), STAGE2_HEADER_BYTES + STAGE2_MAX_BIN_BYTES);
        assert_eq!(
            header_field(&blob, 8),
            (STAGE2_HEADER_BYTES + STAGE2_MAX_BIN_BYTES) as u32
        );
    }

    #[test]
    #[should_panic]
    fn stage2_blob_refuses_an_oversized_bin() {
        stage2_blob(&vec![0u8; STAGE2_MAX_BIN_BYTES + 1]);
    }

    #[test]
    #[should_panic]
    fn stage2_blob_refuses_an_empty_bin() {
        stage2_blob(&[]);
    }

    fn boot_sector(code_byte: u8, table_zone: u8) -> Vec<u8> {
        let mut sector = vec![0u8; SECTOR_BYTES];
        sector[..TABLE_OFFSET].fill(code_byte);
        sector[TABLE_OFFSET..SECTOR_BYTES - 2].fill(table_zone);
        sector[SECTOR_BYTES - 2..].copy_from_slice(&[0x55, 0xAA]);
        sector
    }

    fn single_slice() -> [Option<Entry>; ENTRY_COUNT] {
        let mut entries: [Option<Entry>; ENTRY_COUNT] = [None; ENTRY_COUNT];
        entries[0] = Some(Entry {
            boot: BOOTABLE,
            type_id: FAT16_TYPE,
            start_lba: PARTITION_START_LBA,
            sector_count: 129_024,
        });
        entries
    }

    #[test]
    fn boot_mbr_splices_stage1_code_over_the_generic_bootstrap() {
        let mbr = boot_mbr(&boot_sector(0x90, 0), &single_slice());
        assert_eq!(mbr.len(), SECTOR_BYTES);
        assert!(mbr[..TABLE_OFFSET].iter().all(|&byte| byte == 0x90));
        assert_eq!(mbr[446 + 4], FAT16_TYPE);
        assert_eq!(&mbr[SECTOR_BYTES - 2..], &[0x55, 0xAA]);
    }

    #[test]
    fn boot_mbr_encodes_the_slice_at_the_requested_lba() {
        let mbr = boot_mbr(&boot_sector(0x00, 0), &single_slice());
        let start = u32::from_le_bytes(mbr[446 + 8..446 + 12].try_into().unwrap());
        assert_eq!(start, PARTITION_START_LBA);
        assert_eq!(mbr[446], BOOTABLE);
    }

    #[test]
    #[should_panic(expected = "partition table zone")]
    fn boot_mbr_refuses_code_that_reaches_the_table_zone() {
        boot_mbr(&boot_sector(0x90, 0xCD), &single_slice());
    }

    #[test]
    #[should_panic(expected = "one boot sector")]
    fn boot_mbr_refuses_a_wrong_sized_stage1() {
        boot_mbr(&[0x90u8; SECTOR_BYTES + 512], &single_slice());
    }

    #[test]
    #[should_panic(expected = "0x55AA")]
    fn boot_mbr_refuses_a_stage1_without_the_boot_signature() {
        let mut sector = boot_sector(0x90, 0);
        sector[SECTOR_BYTES - 1] = 0;
        boot_mbr(&sector, &single_slice());
    }
}
