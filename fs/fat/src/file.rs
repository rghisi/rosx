use crate::boot::{BootSector, SECTOR_BYTES};
use crate::device::BlockDevice;
use fs_core::error::FsError;

const SECTOR: usize = SECTOR_BYTES as usize;

pub fn read_file<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    chain: &[u32],
    size: u64,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, FsError> {
    if offset >= size {
        return Ok(0);
    }
    let available = core::cmp::min(buf.len() as u64, size - offset) as usize;
    let mut scratch = [0u8; SECTOR];
    let mut produced = 0usize;
    while produced < available {
        let absolute = offset + produced as u64;
        let sector_offset = (absolute % SECTOR as u64) as usize;
        let remaining = available - produced;
        if sector_offset != 0 || remaining < SECTOR {
            let sector = sector_of(boot, chain, absolute)?;
            device
                .read_block(sector, &mut scratch)
                .map_err(FsError::Io)?;
            let take = core::cmp::min(SECTOR - sector_offset, remaining);
            buf[produced..produced + take]
                .copy_from_slice(&scratch[sector_offset..sector_offset + take]);
            produced += take;
            continue;
        }
        let (first, run) = contiguous_run(boot, chain, absolute, remaining)?;
        let span = run * SECTOR;
        device
            .read_blocks(first, run, &mut buf[produced..produced + span])
            .map_err(FsError::Io)?;
        produced += span;
    }
    Ok(produced)
}

fn sector_of(boot: &BootSector, chain: &[u32], absolute: u64) -> Result<u64, FsError> {
    let cluster = cluster_at(boot, chain, absolute)?;
    let first = boot.first_sector_of_cluster(cluster).ok_or(FsError::Corrupt)?;
    let sector_index = (absolute % cluster_bytes(boot)) / SECTOR as u64;
    first.checked_add(sector_index).ok_or(FsError::Corrupt)
}

fn cluster_at(boot: &BootSector, chain: &[u32], absolute: u64) -> Result<u32, FsError> {
    let index = (absolute / cluster_bytes(boot)) as usize;
    chain.get(index).copied().ok_or(FsError::Corrupt)
}

fn cluster_bytes(boot: &BootSector) -> u64 {
    u64::from(boot.sectors_per_cluster) * SECTOR as u64
}

fn contiguous_run(
    boot: &BootSector,
    chain: &[u32],
    absolute: u64,
    remaining: usize,
) -> Result<(u64, usize), FsError> {
    let sectors_per_cluster = boot.sectors_per_cluster as usize;
    let start_index = (absolute / cluster_bytes(boot)) as usize;
    let mut cluster = *chain.get(start_index).ok_or(FsError::Corrupt)?;
    let cluster_first = boot.first_sector_of_cluster(cluster).ok_or(FsError::Corrupt)?;
    let used = ((absolute % cluster_bytes(boot)) / SECTOR as u64) as usize;
    let budget = remaining / SECTOR;
    let mut run = (sectors_per_cluster - used).min(budget);
    let first = cluster_first + used as u64;
    let mut index = start_index;
    while run < budget {
        let Some(next) = chain.get(index + 1).copied() else {
            break;
        };
        if next != cluster + 1 {
            break;
        }
        cluster = next;
        index += 1;
        run += sectors_per_cluster.min(budget - run);
    }
    Ok((first, run))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fat::fixtures::{fat16_boot, fat16_entry_at, TestDevice};
    use alloc::vec;
    use alloc::vec::Vec;
    use fs_core::error::IoError;

    fn fill_sector(device: &mut TestDevice, sector: u64, start_byte: u8) {
        let at = sector as usize * 512;
        for (index, byte) in device.bytes[at..at + 512].iter_mut().enumerate() {
            *byte = start_byte.wrapping_add(index as u8);
        }
    }

    fn link(device: &mut TestDevice, from: u32, to: u32) {
        let (sector, offset) = fat16_entry_at(from);
        device.put16(sector, offset, to.try_into().expect("fits fat16 entry"));
    }

    fn end_chain(device: &mut TestDevice, cluster: u32) {
        let (sector, offset) = fat16_entry_at(cluster);
        device.put16(sector, offset, 0xFFF8);
    }

    #[test]
    fn reads_a_file_inside_one_cluster() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        end_chain(&mut device, 2);
        fill_sector(&mut device, 49, 0xA0);
        let mut buf = [0u8; 10];
        assert_eq!(read_file(&mut device, &boot, &[2], 10, 0, &mut buf).expect("reads"), 10);
        assert_eq!(buf[0], 0xA0);
        assert_eq!(buf[9], 0xA9);
    }

    #[test]
    fn reads_at_an_offset_inside_a_cluster() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        end_chain(&mut device, 2);
        fill_sector(&mut device, 49, 0x00);
        let mut buf = [0u8; 4];
        assert_eq!(read_file(&mut device, &boot, &[2], 512, 500, &mut buf).expect("reads"), 4);
        assert_eq!(buf, [244, 245, 246, 247]);
    }

    #[test]
    fn reads_a_file_across_two_clusters() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        link(&mut device, 2, 3);
        end_chain(&mut device, 3);
        fill_sector(&mut device, 49, 0x10);
        fill_sector(&mut device, 50, 0x20);
        let mut buf = [0u8; 1_024];
        assert_eq!(read_file(&mut device, &boot, &[2, 3], 1_024, 0, &mut buf).expect("reads"), 1_024);
        assert_eq!(buf[511], 0x0F);
        assert_eq!(buf[512], 0x20);
    }

    #[test]
    fn reads_a_straddle_of_a_cluster_boundary() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        link(&mut device, 2, 3);
        end_chain(&mut device, 3);
        fill_sector(&mut device, 49, 0x00);
        fill_sector(&mut device, 50, 0x40);
        let mut buf = [0u8; 24];
        assert_eq!(read_file(&mut device, &boot, &[2, 3], 1_024, 500, &mut buf).expect("reads"), 24);
        assert_eq!(buf[0], 244);
        assert_eq!(buf[11], 0xFF);
        assert_eq!(buf[12], 0x40);
    }

    #[test]
    fn reads_inside_a_multi_sector_cluster() {
        let boot = BootSector {
            sectors_per_cluster: 4,
            ..fat16_boot()
        };
        let mut device = TestDevice::new(boot.total_sectors);
        link(&mut device, 2, 3);
        end_chain(&mut device, 3);
        for sector in 49..53 {
            for (index, byte) in device
                .bytes
                .iter_mut()
                .enumerate()
                .skip(sector as usize * 512)
                .take(512)
            {
                *byte = (index % 256) as u8;
            }
        }
        let mut buf = [0u8; 16];
        let read = read_file(&mut device, &boot, &[2, 3], 2_048, 1_020, &mut buf).expect("reads");
        assert_eq!(read, 16);
        assert_eq!(buf[0], (50 * 512 + 508) as u8);
        assert_eq!(buf[3], (50 * 512 + 511) as u8);
        assert_eq!(buf[4], (51 * 512) as u8);
        assert_eq!(buf[15], (51 * 512 + 11) as u8);
    }

    #[test]
    fn truncated_buffers_and_past_eof_reads() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        end_chain(&mut device, 2);
        fill_sector(&mut device, 49, 0x00);
        let mut buf = [0u8; 100];
        assert_eq!(read_file(&mut device, &boot, &[2], 60, 0, &mut buf).expect("reads"), 60);
        assert_eq!(read_file(&mut device, &boot, &[2], 60, 55, &mut buf).expect("reads"), 5);
        assert_eq!(read_file(&mut device, &boot, &[2], 60, 60, &mut buf).expect("at eof"), 0);
        assert_eq!(read_file(&mut device, &boot, &[2], 60, 61, &mut buf).expect("past eof"), 0);
    }

    #[test]
    fn empty_files_need_no_clusters() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(1);
        assert_eq!(read_file(&mut device, &boot, &[], 0, 0, &mut [0u8; 16]).expect("empty"), 0);
    }

    #[test]
    fn sizes_beyond_the_cluster_chain_are_corrupt() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        end_chain(&mut device, 2);
        let mut buf = [0u8; 600];
        assert_eq!(read_file(&mut device, &boot, &[2], 600, 0, &mut buf), Err(FsError::Corrupt));
    }

    #[test]
    fn a_chain_shorter_than_the_size_is_corrupt() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let mut buf = [0u8; 8];
        assert_eq!(read_file(&mut device, &boot, &[], 8, 0, &mut buf), Err(FsError::Corrupt));
    }

    #[test]
    fn device_errors_surfaces_as_io() {
        let boot = fat16_boot();
        let mut stub = TinyDevice;
        assert_eq!(
            read_file(&mut stub, &boot, &[2], 8, 0, &mut [0u8; 8]),
            Err(FsError::Io(IoError::Device))
        );
    }

    struct TinyDevice;

    impl BlockDevice for TinyDevice {
        fn block_count(&self) -> u64 {
            1
        }

        fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
            let _ = (block, buf);
            Err(IoError::Device)
        }

        fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError> {
            let _ = (block, buf);
            Err(IoError::Device)
        }

        fn flush(&mut self) -> Result<(), IoError> {
            Err(IoError::Flush)
        }
    }

    #[test]
    fn zero_length_buffers_read_nothing() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        end_chain(&mut device, 2);
        assert_eq!(read_file(&mut device, &boot, &[2], 10, 0, &mut []).expect("nothing"), 0);
    }

    fn link_run(device: &mut TestDevice, clusters: &[u32]) {
        for pair in clusters.windows(2) {
            link(device, pair[0], pair[1]);
        }
        end_chain(device, *clusters.last().expect("non-empty run"));
    }

    #[test]
    fn an_aligned_contiguous_chain_is_read_in_one_device_run() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let clusters: Vec<u32> = (2..18).collect();
        link_run(&mut device, &clusters);
        for cluster in &clusters {
            fill_sector(&mut device, 49 + u64::from(cluster - 2), u8::try_from(*cluster).expect("u8"));
        }
        let mut buf = vec![0u8; 16 * 512];
        assert_eq!(
            read_file(&mut device, &boot, &clusters, 16 * 512, 0, &mut buf).expect("reads"),
            16 * 512
        );
        assert_eq!(device.run_log, vec![(49, 16)]);
        assert_eq!(device.read_log, (49..65).collect::<Vec<u64>>());
        assert_eq!(buf[0], 2);
        assert_eq!(buf[512], 3);
    }

    #[test]
    fn a_scattered_chain_falls_back_to_one_run_per_cluster() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        link(&mut device, 2, 9);
        link(&mut device, 9, 16);
        end_chain(&mut device, 16);
        for cluster in [2u32, 9, 16] {
            fill_sector(&mut device, 49 + u64::from(cluster - 2), u8::try_from(cluster).expect("u8"));
        }
        let mut buf = [0u8; 3 * 512];
        assert_eq!(
            read_file(&mut device, &boot, &[2, 9, 16], 3 * 512, 0, &mut buf).expect("reads"),
            3 * 512
        );
        assert_eq!(device.run_log, vec![(49, 1), (56, 1), (63, 1)]);
        assert_eq!(buf[0], 2);
        assert_eq!(buf[512], 9);
        assert_eq!(buf[1_024], 16);
    }

    #[test]
    fn a_run_splits_at_the_first_cluster_hole() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        link_run(&mut device, &[2, 3]);
        link_run(&mut device, &[7, 8]);
        let chain = [2u32, 3, 7, 8];
        for cluster in chain {
            fill_sector(&mut device, 49 + u64::from(cluster - 2), u8::try_from(cluster).expect("u8"));
        }
        let mut buf = vec![0u8; 4 * 512];
        assert_eq!(read_file(&mut device, &boot, &chain, 4 * 512, 0, &mut buf).expect("reads"), 4 * 512);
        assert_eq!(device.run_log, vec![(49, 2), (54, 2)]);
    }

    #[test]
    fn misaligned_head_and_tail_stay_single_sector_reads_outside_the_run() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        link_run(&mut device, &[2, 3, 4]);
        for cluster in [2u32, 3, 4] {
            fill_sector(&mut device, 49 + u64::from(cluster - 2), u8::try_from(cluster).expect("u8"));
        }
        let chain = [2u32, 3, 4];
        let mut buf = [0u8; 1_200];
        assert_eq!(
            read_file(&mut device, &boot, &chain, 3 * 512, 100, &mut buf).expect("reads"),
            1_200
        );
        assert_eq!(device.run_log, vec![(50, 1)]);
        assert_eq!(device.read_log, vec![49, 50, 51]);
    }

    #[test]
    fn a_multi_sector_cluster_yields_a_run_within_and_across_clusters() {
        let boot = BootSector {
            sectors_per_cluster: 4,
            ..fat16_boot()
        };
        let mut device = TestDevice::new(boot.total_sectors);
        link_run(&mut device, &[2, 3]);
        let chain = [2u32, 3];
        let mut buf = vec![0u8; 8 * 512];
        assert_eq!(
            read_file(&mut device, &boot, &chain, 8 * 512, 0, &mut buf).expect("reads"),
            8 * 512
        );
        assert_eq!(device.run_log, vec![(49, 8)]);
        let mut partial = vec![0u8; 8 * 512];
        assert_eq!(
            read_file(&mut device, &boot, &chain, 8 * 512, 1_536, &mut partial).expect("tail"),
            5 * 512
        );
        assert_eq!(device.run_log, vec![(49, 8), (52, 5)]);
    }
}
