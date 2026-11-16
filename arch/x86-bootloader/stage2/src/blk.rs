use fs_core::device::BlockDevice;
use fs_core::error::IoError;
use rosx_boot_common::bios::Regs16;
use rosx_boot_common::biosdisk::BIOS_CHS_CYLINDER_LIMIT;
use rosx_boot_common::biosdisk::BOUNCE_ADDRESS;
use rosx_boot_common::biosdisk::BOUNCE_SELECTOR;
use rosx_boot_common::biosdisk::BiosDisk;
use rosx_boot_common::biosdisk::ChsGeometry;
use rosx_boot_common::biosdisk::DAP_ADDRESS;
use rosx_boot_common::biosdisk::EDD_MARKER;
use rosx_boot_common::biosdisk::EDD_PROBE_MAGIC;
use rosx_boot_common::biosdisk::GEOMETRY_PACKET_ADDRESS;
use rosx_boot_common::biosdisk::GEOMETRY_PACKET_SIZE;
use rosx_boot_common::biosdisk::GEOMETRY_SECTOR_COUNT_OFFSET;
use rosx_boot_common::biosdisk::INT13;
use rosx_boot_common::biosdisk::PARTITION_BASE_LBA;
use rosx_boot_common::biosdisk::SECTOR_BYTES;
use rosx_boot_common::biosdisk::chs_registers;
use rosx_boot_common::biosdisk::dap_bytes;
use rosx_boot_common::biosdisk::read_plan;
use rosx_boot_common::biosdisk::to_chs;
use rxpart::mbr::Entry;
use rxpart::mbr::Table;
use rxpart::mbr::TableState;
use rxpart::mbr::parse_mbr;

use crate::thunk::BiosThunk;

const READ_ATTEMPTS: usize = 3;

pub enum DiskMedia {
    Lba,
    Chs(ChsGeometry),
}

pub struct BiosBlockDevice<'a> {
    thunk: &'a BiosThunk,
    drive: u8,
    partition_sectors: u64,
    media: DiskMedia,
}

impl<'a> BiosBlockDevice<'a> {
    pub fn bring_up(thunk: &'a BiosThunk, drive: u8) -> Option<Self> {
        let (media, disk_sectors) = probe_media(thunk, drive)?;
        let device = Self {
            thunk,
            drive,
            partition_sectors: 0,
            media,
        };
        let mut mbr = [0u8; SECTOR_BYTES];
        device.transfer_raw(0, &mut mbr).ok()?;
        let entry = boot_slice(&parse_mbr(&mbr, disk_sectors))?;
        Some(Self {
            partition_sectors: u64::from(entry.sector_count),
            ..device
        })
    }

    pub fn media_name(&self) -> &'static str {
        match self.media {
            DiskMedia::Lba => "lba",
            DiskMedia::Chs(_) => "chs",
        }
    }

    pub fn slice_sectors(&self) -> u64 {
        self.partition_sectors
    }

    fn transfer_raw(&self, lba: u64, dest: &mut [u8]) -> Result<(), IoError> {
        if dest.len() != SECTOR_BYTES {
            return Err(IoError::Device);
        }
        match self.media {
            DiskMedia::Lba => self.transfer_lba(lba, 1)?,
            DiskMedia::Chs(geometry) => self.transfer_chs(geometry, lba)?,
        }
        copy_bounce_out(dest);
        Ok(())
    }

    fn transfer_lba(&self, lba: u64, sectors: usize) -> Result<(), IoError> {
        for attempt in 0..READ_ATTEMPTS {
            if attempt > 0 {
                reset_controller(self.thunk, self.drive);
            }
            write_dap(lba, sectors);
            let mut regs = Regs16::new(0x4200, 0, 0, u16::from(self.drive));
            regs.si = DAP_ADDRESS as u16;
            self.thunk.call(INT13, &mut regs);
            if !regs.carry() {
                return Ok(());
            }
        }
        Err(IoError::Device)
    }

    fn transfer_chs(&self, geometry: ChsGeometry, lba: u64) -> Result<(), IoError> {
        let address = to_chs(&geometry, lba).ok_or(IoError::Device)?;
        let (cx, dh) = chs_registers(&address);
        for attempt in 0..READ_ATTEMPTS {
            if attempt > 0 {
                reset_controller(self.thunk, self.drive);
            }
            let mut regs = Regs16::new(0x0201, 0, cx, u16::from(self.drive));
            regs.dx |= u16::from(dh) << 8;
            regs.es = BOUNCE_SELECTOR;
            self.thunk.call(INT13, &mut regs);
            if !regs.carry() {
                return Ok(());
            }
        }
        Err(IoError::Device)
    }
}

impl BlockDevice for BiosBlockDevice<'_> {
    fn block_count(&self) -> u64 {
        self.partition_sectors
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
        self.read_blocks(block, 1, buf)
    }

    fn read_blocks(&mut self, first: u64, count: usize, dest: &mut [u8]) -> Result<(), IoError> {
        if dest.len() != count * SECTOR_BYTES {
            return Err(IoError::Device);
        }
        let plan = read_plan(&self.disk(), first, count).ok_or(IoError::Device)?;
        let mut cursor = 0usize;
        for chunk in plan {
            match self.media {
                DiskMedia::Lba => {
                    self.transfer_lba(chunk.lba, chunk.sectors)?;
                    let span = chunk.sectors * SECTOR_BYTES;
                    copy_bounce_out(&mut dest[cursor..cursor + span]);
                    cursor += span;
                }
                DiskMedia::Chs(geometry) => {
                    for offset in 0..chunk.sectors {
                        self.transfer_chs(geometry, chunk.lba + offset as u64)?;
                        copy_bounce_out(&mut dest[cursor..cursor + SECTOR_BYTES]);
                        cursor += SECTOR_BYTES;
                    }
                }
            }
        }
        Ok(())
    }

    fn write_block(&mut self, _block: u64, _buf: &[u8]) -> Result<(), IoError> {
        Err(IoError::Device)
    }

    fn flush(&mut self) -> Result<(), IoError> {
        Ok(())
    }
}

impl BiosBlockDevice<'_> {
    fn disk(&self) -> BiosDisk {
        BiosDisk {
            drive: self.drive,
            partition_sectors: self.partition_sectors,
        }
    }
}

fn probe_media(thunk: &BiosThunk, drive: u8) -> Option<(DiskMedia, u64)> {
    let mut probe = Regs16::new(0x4100, EDD_PROBE_MAGIC, 0, u16::from(drive));
    thunk.call(INT13, &mut probe);
    if !probe.carry() && probe.bx == EDD_MARKER {
        write_u16_volatile(GEOMETRY_PACKET_ADDRESS, GEOMETRY_PACKET_SIZE);
        let mut packet = Regs16::new(0x4800, 0, 0, u16::from(drive));
        packet.si = GEOMETRY_PACKET_ADDRESS as u16;
        thunk.call(INT13, &mut packet);
        if !packet.carry() {
            let total = read_u64_volatile(GEOMETRY_PACKET_ADDRESS + GEOMETRY_SECTOR_COUNT_OFFSET);
            if total > 0 {
                return Some((DiskMedia::Lba, total));
            }
        }
    }
    let mut classic = Regs16::new(0x0800, 0, 0, u16::from(drive));
    thunk.call(INT13, &mut classic);
    if classic.carry() {
        return None;
    }
    let geometry = ChsGeometry::from_int13_registers(classic.cx as u8, (classic.dx >> 8) as u8)?;
    let addressable = u64::from(geometry.sectors_per_track)
        .checked_mul(u64::from(geometry.heads))?
        .checked_mul(BIOS_CHS_CYLINDER_LIMIT)?;
    Some((DiskMedia::Chs(geometry), addressable))
}

fn boot_slice(table: &Table) -> Option<Entry> {
    if table.state != TableState::Valid {
        return None;
    }
    table
        .entries
        .iter()
        .flatten()
        .find(|entry| u64::from(entry.start_lba) == PARTITION_BASE_LBA)
        .copied()
}

fn write_dap(lba: u64, sectors: usize) {
    let bytes = dap_bytes(sectors, lba, BOUNCE_ADDRESS);
    for (index, word) in bytes.as_chunks::<4>().0.iter().enumerate() {
        write_u32_volatile(DAP_ADDRESS + index * 4, u32::from_le_bytes(*word));
    }
}

fn copy_bounce_out(dest: &mut [u8]) {
    for (index, word) in dest.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let value = read_u32_volatile(BOUNCE_ADDRESS + index * 4);
        *word = value.to_le_bytes();
    }
}

fn reset_controller(thunk: &BiosThunk, drive: u8) {
    let mut regs = Regs16::new(0x0000, 0, 0, u16::from(drive));
    thunk.call(INT13, &mut regs);
}

fn read_u32_volatile(address: usize) -> u32 {
    unsafe { (address as *const u32).read_volatile() }
}

fn read_u64_volatile(address: usize) -> u64 {
    u64::from(read_u32_volatile(address)) | (u64::from(read_u32_volatile(address + 4)) << 32)
}

fn write_u32_volatile(address: usize, value: u32) {
    unsafe { (address as *mut u32).write_volatile(value) }
}

fn write_u16_volatile(address: usize, value: u16) {
    unsafe { (address as *mut u16).write_volatile(value) }
}
