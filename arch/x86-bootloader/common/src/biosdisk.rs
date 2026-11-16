pub const SECTOR_BYTES: usize = 512;
pub const PARTITION_BASE_LBA: u64 = 2048;
pub const MAX_CHUNK_SECTORS: usize = 64;
pub const BOUNCE_SELECTOR: u16 = 0x4000;
pub const BOUNCE_ADDRESS: usize = 0x0004_0000;
pub const BOUNCE_BYTES: usize = 64 * SECTOR_BYTES;
pub const HEAP_ADDRESS: usize = BOUNCE_ADDRESS + BOUNCE_BYTES;
pub const HEAP_BYTES: usize = 64 * 1024;
pub const DAP_ADDRESS: usize = 0x7C10;
pub const DAP_BYTES: usize = 16;
pub const GEOMETRY_PACKET_ADDRESS: usize = 0x0580;
pub const GEOMETRY_PACKET_SIZE: u16 = 0x0040;
pub const GEOMETRY_SECTOR_COUNT_OFFSET: usize = 16;
pub const INT13: u8 = 0x13;
pub const EDD_PROBE_MAGIC: u16 = 0x55AA;
pub const EDD_MARKER: u16 = 0xAA55;
pub const HARD_DRIVE_BASE: u8 = 0x80;
pub const BIOS_CHS_CYLINDER_LIMIT: u64 = 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BiosDisk {
    pub drive: u8,
    pub partition_sectors: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TransferChunk {
    pub lba: u64,
    pub sectors: usize,
}

pub struct ChunkPlan {
    next_lba: u64,
    end_lba: u64,
}

impl Iterator for ChunkPlan {
    type Item = TransferChunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_lba >= self.end_lba {
            return None;
        }
        let remaining = self.end_lba - self.next_lba;
        let sectors = remaining.min(MAX_CHUNK_SECTORS as u64) as usize;
        let chunk = TransferChunk {
            lba: self.next_lba,
            sectors,
        };
        self.next_lba += sectors as u64;
        Some(chunk)
    }
}

pub fn read_plan(disk: &BiosDisk, first_sector: u64, sector_count: usize) -> Option<ChunkPlan> {
    if sector_count == 0 {
        return None;
    }
    let end = first_sector.checked_add(sector_count as u64)?;
    if end > disk.partition_sectors {
        return None;
    }
    Some(ChunkPlan {
        next_lba: PARTITION_BASE_LBA.checked_add(first_sector)?,
        end_lba: PARTITION_BASE_LBA.checked_add(end)?,
    })
}

pub fn dap_bytes(sectors: usize, lba: u64, buffer_address: usize) -> [u8; DAP_BYTES] {
    let mut dap = [0u8; DAP_BYTES];
    dap[0] = DAP_BYTES as u8;
    dap[2..4].copy_from_slice(&(sectors as u16).to_le_bytes());
    dap[4..6].copy_from_slice(&(buffer_address as u16).to_le_bytes());
    dap[6..8].copy_from_slice(&((buffer_address >> 4) as u16).to_le_bytes());
    dap[8..16].copy_from_slice(&lba.to_le_bytes());
    dap
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ChsGeometry {
    pub sectors_per_track: u8,
    pub heads: u8,
}

impl ChsGeometry {
    pub fn from_int13_registers(cl: u8, dh: u8) -> Option<Self> {
        let sectors_per_track = cl & 0x3F;
        let heads = dh.checked_add(1)?;
        if sectors_per_track == 0 {
            return None;
        }
        Some(Self {
            sectors_per_track,
            heads,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ChsAddress {
    pub cylinder: u16,
    pub head: u8,
    pub sector: u8,
}

pub fn to_chs(geometry: &ChsGeometry, lba: u64) -> Option<ChsAddress> {
    let per_track = u64::from(geometry.sectors_per_track);
    let per_cylinder = per_track.checked_mul(u64::from(geometry.heads))?;
    let cylinder = lba / per_cylinder;
    let within_cylinder = lba % per_cylinder;
    let head = (within_cylinder / per_track) as u8;
    let sector = (within_cylinder % per_track) as u8 + 1;
    if cylinder > u64::from(u16::MAX) || sector > 63 {
        return None;
    }
    Some(ChsAddress {
        cylinder: cylinder as u16,
        head,
        sector,
    })
}

pub fn chs_registers(address: &ChsAddress) -> (u16, u8) {
    let cl = address.sector | (((address.cylinder >> 8) & 0x3) as u8) << 6;
    let cx = (u16::from(address.cylinder as u8) << 8) | u16::from(cl);
    (cx, address.head)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISK: BiosDisk = BiosDisk {
        drive: 0x80,
        partition_sectors: 4096,
    };

    fn chunks(first: u64, count: usize) -> Vec<TransferChunk> {
        read_plan(&DISK, first, count).unwrap().collect()
    }

    #[test]
    fn chunks_start_at_the_partition_base() {
        assert_eq!(
            chunks(0, 2),
            vec![TransferChunk {
                lba: 2048,
                sectors: 2
            }]
        );
    }

    #[test]
    fn a_long_run_splits_at_the_sixty_four_sector_ceiling() {
        let plan = chunks(0, 130);
        assert_eq!(
            plan,
            vec![
                TransferChunk {
                    lba: 2048,
                    sectors: 64
                },
                TransferChunk {
                    lba: 2112,
                    sectors: 64
                },
                TransferChunk {
                    lba: 2176,
                    sectors: 2
                },
            ]
        );
    }

    #[test]
    fn a_run_reaching_the_volume_end_stops_exactly_at_it() {
        let plan = chunks(4090, 6);
        assert_eq!(plan.last().unwrap().lba + plan.last().unwrap().sectors as u64, 2048 + 4096);
        let total: usize = plan.iter().map(|chunk| chunk.sectors).sum();
        assert_eq!(total, 6);
    }

    #[test]
    fn a_run_past_the_volume_end_is_refused_before_any_transfer() {
        assert!(read_plan(&DISK, 4096, 1).is_none());
        assert!(read_plan(&DISK, 4090, 7).is_none());
        assert!(read_plan(&DISK, 0, 0).is_none());
        assert!(read_plan(&DISK, u64::MAX, 2).is_none());
    }

    #[test]
    fn the_dap_carries_size_count_buffer_and_lba_at_their_frozen_offsets() {
        let dap = dap_bytes(64, 0x0000_0123_4567_89AB, BOUNCE_ADDRESS);
        assert_eq!(dap[0], 16);
        assert_eq!(dap[1], 0);
        assert_eq!(&dap[2..4], &64u16.to_le_bytes());
        assert_eq!(&dap[4..6], &0u16.to_le_bytes());
        assert_eq!(&dap[6..8], &0x4000u16.to_le_bytes());
        assert_eq!(&dap[8..16], &0x0000_0123_4567_89ABu64.to_le_bytes());
    }

    #[test]
    fn the_geometry_decodes_the_int13_register_conventions() {
        assert_eq!(
            ChsGeometry::from_int13_registers(0xBF, 0xFE),
            Some(ChsGeometry {
                sectors_per_track: 63,
                heads: 255
            })
        );
        assert_eq!(ChsGeometry::from_int13_registers(0x00, 0xFE), None);
        assert_eq!(ChsGeometry::from_int13_registers(0x3F, 0xFF), None);
    }

    #[test]
    fn lba_maps_onto_the_classic_chs_grid() {
        let geometry = ChsGeometry {
            sectors_per_track: 63,
            heads: 255,
        };
        assert_eq!(
            to_chs(&geometry, 0),
            Some(ChsAddress {
                cylinder: 0,
                head: 0,
                sector: 1
            })
        );
        assert_eq!(
            to_chs(&geometry, 62),
            Some(ChsAddress {
                cylinder: 0,
                head: 0,
                sector: 63
            })
        );
        assert_eq!(
            to_chs(&geometry, 63),
            Some(ChsAddress {
                cylinder: 0,
                head: 1,
                sector: 1
            })
        );
        assert_eq!(
            to_chs(&geometry, 63 * 255),
            Some(ChsAddress {
                cylinder: 1,
                head: 0,
                sector: 1
            })
        );
    }

    #[test]
    fn chs_registers_pack_the_int13_cx_and_dh_layout() {
        let address = ChsAddress {
            cylinder: 0x2AB,
            head: 0xCD,
            sector: 3,
        };
        let (cx, dh) = chs_registers(&address);
        assert_eq!(cx & 0x3F, 3);
        assert_eq!((cx >> 6) & 0x3, 2);
        assert_eq!((cx >> 8) & 0xFF, 0xAB);
        assert_eq!(dh, 0xCD);
    }

    #[test]
    fn chs_beyond_the_cylinder_field_is_refused() {
        let geometry = ChsGeometry {
            sectors_per_track: 63,
            heads: 255,
        };
        let too_wide = 63_u64 * 255 * (u64::from(u16::MAX) + 1);
        assert_eq!(to_chs(&geometry, too_wide), None);
    }
}
