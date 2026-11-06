use crate::boot::BootSector;
use crate::device::BlockDevice;
use crate::fat;
use fs_core::error::FsError;

pub fn read_file<D: BlockDevice>(
    device: &mut D,
    boot: &BootSector,
    start_cluster: u32,
    size: u64,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, FsError> {
    if offset >= size {
        return Ok(0);
    }
    let available = core::cmp::min(buf.len() as u64, size - offset) as usize;
    let chain = fat::cluster_chain(device, boot, start_cluster)?;
    let cluster_bytes = u64::from(boot.sectors_per_cluster) * u64::from(boot.bytes_per_sector);
    let mut block = [0u8; 512];
    let mut produced = 0usize;
    while produced < available {
        let absolute = offset + produced as u64;
        let chain_index = (absolute / cluster_bytes) as usize;
        let cluster = *chain.get(chain_index).ok_or(FsError::Corrupt)?;
        let sector = boot
            .first_sector_of_cluster(cluster)
            .and_then(|first| {
                let sector_index =
                    (absolute % cluster_bytes) / u64::from(boot.bytes_per_sector);
                first.checked_add(sector_index)
            })
            .ok_or(FsError::Corrupt)?;
        device.read_block(sector, &mut block).map_err(FsError::Io)?;
        let sector_offset =
            (absolute % u64::from(boot.bytes_per_sector)) as usize;
        let take = core::cmp::min(
            block.len() - sector_offset,
            available - produced,
        );
        buf[produced..produced + take].copy_from_slice(&block[sector_offset..sector_offset + take]);
        produced += take;
    }
    Ok(produced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fat::fixtures::{fat16_boot, fat16_entry_at, TestDevice};
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
        assert_eq!(read_file(&mut device, &boot, 2, 10, 0, &mut buf).expect("reads"), 10);
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
        assert_eq!(read_file(&mut device, &boot, 2, 512, 500, &mut buf).expect("reads"), 4);
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
        assert_eq!(read_file(&mut device, &boot, 2, 1_024, 0, &mut buf).expect("reads"), 1_024);
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
        assert_eq!(read_file(&mut device, &boot, 2, 1_024, 500, &mut buf).expect("reads"), 24);
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
        let read = read_file(&mut device, &boot, 2, 2_048, 1_020, &mut buf).expect("reads");
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
        assert_eq!(read_file(&mut device, &boot, 2, 60, 0, &mut buf).expect("reads"), 60);
        assert_eq!(read_file(&mut device, &boot, 2, 60, 55, &mut buf).expect("reads"), 5);
        assert_eq!(read_file(&mut device, &boot, 2, 60, 60, &mut buf).expect("at eof"), 0);
        assert_eq!(read_file(&mut device, &boot, 2, 60, 61, &mut buf).expect("past eof"), 0);
    }

    #[test]
    fn empty_files_need_no_clusters() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(1);
        assert_eq!(read_file(&mut device, &boot, 0, 0, 0, &mut [0u8; 16]).expect("empty"), 0);
    }

    #[test]
    fn sizes_beyond_the_cluster_chain_are_corrupt() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        end_chain(&mut device, 2);
        let mut buf = [0u8; 600];
        assert_eq!(read_file(&mut device, &boot, 2, 600, 0, &mut buf), Err(FsError::Corrupt));
    }

    #[test]
    fn chain_corruption_surfaces_from_reads() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        let (sector, offset) = fat16_entry_at(2);
        device.put16(sector, offset, 0);
        let mut buf = [0u8; 8];
        assert_eq!(read_file(&mut device, &boot, 2, 8, 0, &mut buf), Err(FsError::Corrupt));
    }

    #[test]
    fn device_errors_surfaces_as_io() {
        let boot = fat16_boot();
        let mut stub = TinyDevice;
        assert_eq!(
            read_file(&mut stub, &boot, 2, 8, 0, &mut [0u8; 8]),
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
    }

    #[test]
    fn zero_length_buffers_read_nothing() {
        let boot = fat16_boot();
        let mut device = TestDevice::new(boot.total_sectors);
        end_chain(&mut device, 2);
        assert_eq!(read_file(&mut device, &boot, 2, 10, 0, &mut []).expect("nothing"), 0);
    }
}
