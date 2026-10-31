use alloc::string::{String, ToString};
use ata_protocol::wire::{AtaError, LBA28_MAX, SECTOR_BYTES};
use usrlib::error::PortError;

pub const IDENTIFY_WORD_COUNT: usize = 256;
pub const WORDS_PER_SECTOR: usize = IDENTIFY_WORD_COUNT;

const DATA_PORT: u16 = 0x1F0;
const ERROR_REGISTER_PORT: u16 = 0x1F1;
const SECTOR_COUNT_PORT: u16 = 0x1F2;
const LBA_LOW_PORT: u16 = 0x1F3;
const LBA_MID_PORT: u16 = 0x1F4;
const LBA_HIGH_PORT: u16 = 0x1F5;
const DRIVE_HEAD_PORT: u16 = 0x1F6;
const COMMAND_PORT: u16 = 0x1F7;
const ALTERNATE_STATUS_PORT: u16 = 0x3F6;

const STATUS_BUSY: u8 = 0x80;
const STATUS_DATA_REQUEST: u8 = 0x08;
const STATUS_ERROR: u8 = 0x01;

const CHS_LBA_COMMAND_BASE: u8 = 0xE0;
const CHS_DRIVE_HEAD_BASE: u8 = 0xA0;
const SOFT_RESET_BIT: u8 = 0x04;
const READ_SECTORS_COMMAND: u8 = 0x20;
const WRITE_SECTORS_COMMAND: u8 = 0x30;
const FLUSH_CACHE_COMMAND: u8 = 0xE7;
const IDENTIFY_DEVICE_COMMAND: u8 = 0xEC;

const STATUS_SPINS_BEFORE_SLEEP: usize = 10_000;
const SLEEP_TICKS_BEFORE_BUSY: u64 = 250;

const FLOATING_BUS_BYTE: u8 = 0xFF;

const MODEL_FIRST_WORD: usize = 27;
const MODEL_LAST_WORD: usize = 46;
const FIRMWARE_FIRST_WORD: usize = 23;
const FIRMWARE_LAST_WORD: usize = 26;
const CAPACITY_LOW_WORD: usize = 60;
const CAPACITY_HIGH_WORD: usize = 61;
const CHS_SECTORS_PER_TRACK_WORD: usize = 1;
const CHS_HEADS_WORD: usize = 3;
const CHS_CYLINDERS_WORD: usize = 6;

pub trait Bus {
    fn read_byte(&mut self, port: u16) -> Result<u8, PortError>;
    fn write_byte(&mut self, port: u16, value: u8) -> Result<(), PortError>;
    fn read_words(&mut self, port: u16, destination: &mut [u16]) -> Result<(), PortError>;
    fn write_words(&mut self, port: u16, source: &[u16]) -> Result<(), PortError>;
    fn sleep_ms(&mut self, millis: u64);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DriveFault {
    NotFound,
    BadAddress,
    Busy,
    Device { error_register: u8, status: u8 },
    Denied,
}

impl DriveFault {
    pub fn wire_error(&self) -> (AtaError, u8, u8) {
        match self {
            DriveFault::NotFound => (AtaError::NotFound, 0, 0),
            DriveFault::BadAddress => (AtaError::BadAddress, 0, 0),
            DriveFault::Busy => (AtaError::Busy, 0, 0),
            DriveFault::Denied => (AtaError::Denied, 0, 0),
            DriveFault::Device {
                error_register,
                status,
            } => (AtaError::Device, *error_register, *status),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub model: String,
    pub firmware_rev: String,
    pub sectors: u64,
}

pub struct AtaDrive<B: Bus> {
    bus: B,
    drive: u8,
    identity: Option<Identity>,
}

impl<B: Bus> AtaDrive<B> {
    pub fn new(bus: B, drive: u8) -> AtaDrive<B> {
        AtaDrive {
            bus,
            drive,
            identity: None,
        }
    }

    pub fn identity(&self) -> Option<&Identity> {
        self.identity.as_ref()
    }

    pub fn probe(&mut self) -> Result<Identity, DriveFault> {
        let first_status = self.bus.read_byte(COMMAND_PORT).map_err(|_| DriveFault::Denied)?;
        if first_status == FLOATING_BUS_BYTE {
            return Err(DriveFault::NotFound);
        }
        self.soft_reset()?;
        self.select_drive(CHS_DRIVE_HEAD_BASE)?;
        if !self.signature_is_ata_disk()? {
            return Err(DriveFault::NotFound);
        }
        self.bus
            .write_byte(COMMAND_PORT, IDENTIFY_DEVICE_COMMAND)
            .map_err(|_| DriveFault::Denied)?;
        self.settle()?;
        self.wait_ready(true)?;
        let mut block = [0u16; IDENTIFY_WORD_COUNT];
        self.bus
            .read_words(DATA_PORT, &mut block)
            .map_err(|_| DriveFault::Denied)?;
        let identity = parse_identity(&block);
        if identity.sectors == 0 {
            return Err(DriveFault::NotFound);
        }
        self.identity = Some(identity.clone());
        Ok(identity)
    }

    pub fn read_sectors(&mut self, lba: u32, count: u8, destination: &mut [u8]) -> Result<(), DriveFault> {
        self.guard(lba, count)?;
        if destination.len() < count as usize * SECTOR_BYTES {
            return Err(DriveFault::BadAddress);
        }
        self.stage_transfer(lba, count, READ_SECTORS_COMMAND)?;
        let mut words = [0u16; WORDS_PER_SECTOR];
        for sector in 0..count as usize {
            self.wait_ready(true)?;
            self.bus
                .read_words(DATA_PORT, &mut words)
                .map_err(|_| DriveFault::Denied)?;
            let target = &mut destination[sector * SECTOR_BYTES..(sector + 1) * SECTOR_BYTES];
            for (index, word) in words.iter().enumerate() {
                target[index * 2] = *word as u8;
                target[index * 2 + 1] = (*word >> 8) as u8;
            }
        }
        self.wait_ready(false)
    }

    pub fn write_sectors(&mut self, lba: u32, data: &[u8]) -> Result<(), DriveFault> {
        if data.is_empty() || data.len() % SECTOR_BYTES != 0 {
            return Err(DriveFault::BadAddress);
        }
        let count = data.len() / SECTOR_BYTES;
        self.guard(lba, count as u8)?;
        self.stage_transfer(lba, count as u8, WRITE_SECTORS_COMMAND)?;
        let mut words = [0u16; WORDS_PER_SECTOR];
        for sector in 0..count {
            self.wait_ready(true)?;
            let source = &data[sector * SECTOR_BYTES..(sector + 1) * SECTOR_BYTES];
            for (index, word) in words.iter_mut().enumerate() {
                *word = u16::from_le_bytes([source[index * 2], source[index * 2 + 1]]);
            }
            self.bus
                .write_words(DATA_PORT, &words)
                .map_err(|_| DriveFault::Denied)?;
            self.wait_ready(false)?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), DriveFault> {
        self.identity.as_ref().ok_or(DriveFault::NotFound)?;
        self.bus
            .write_byte(COMMAND_PORT, FLUSH_CACHE_COMMAND)
            .map_err(|_| DriveFault::Denied)?;
        self.settle()?;
        self.wait_ready(false)
    }

    fn guard(&self, lba: u32, count: u8) -> Result<(), DriveFault> {
        let identity = self.identity.as_ref().ok_or(DriveFault::NotFound)?;
        if lba as u64 + count as u64 > identity.sectors {
            return Err(DriveFault::BadAddress);
        }
        Ok(())
    }

    fn stage_transfer(&mut self, lba: u32, count: u8, command: u8) -> Result<(), DriveFault> {
        self.bus
            .write_byte(SECTOR_COUNT_PORT, count)
            .map_err(|_| DriveFault::Denied)?;
        self.bus
            .write_byte(LBA_LOW_PORT, lba as u8)
            .map_err(|_| DriveFault::Denied)?;
        self.bus
            .write_byte(LBA_MID_PORT, (lba >> 8) as u8)
            .map_err(|_| DriveFault::Denied)?;
        self.bus
            .write_byte(LBA_HIGH_PORT, (lba >> 16) as u8)
            .map_err(|_| DriveFault::Denied)?;
        self.select_drive(CHS_LBA_COMMAND_BASE | (((lba >> 24) & 0x0F) as u8))?;
        self.bus
            .write_byte(COMMAND_PORT, command)
            .map_err(|_| DriveFault::Denied)?;
        self.settle()
    }

    fn soft_reset(&mut self) -> Result<(), DriveFault> {
        self.bus
            .write_byte(ALTERNATE_STATUS_PORT, SOFT_RESET_BIT)
            .map_err(|_| DriveFault::Denied)?;
        self.bus
            .write_byte(ALTERNATE_STATUS_PORT, 0)
            .map_err(|_| DriveFault::Denied)?;
        self.settle()?;
        self.wait_ready(false)
    }

    fn select_drive(&mut self, head_nibble_base: u8) -> Result<(), DriveFault> {
        self.bus
            .write_byte(DRIVE_HEAD_PORT, head_nibble_base | (self.drive << 4))
            .map_err(|_| DriveFault::Denied)?;
        self.settle()?;
        self.wait_ready(false)
    }

    fn settle(&mut self) -> Result<(), DriveFault> {
        for _ in 0..4 {
            self.bus
                .read_byte(ALTERNATE_STATUS_PORT)
                .map_err(|_| DriveFault::Denied)?;
        }
        Ok(())
    }

    fn signature_is_ata_disk(&mut self) -> Result<bool, DriveFault> {
        let sector_count = self
            .bus
            .read_byte(SECTOR_COUNT_PORT)
            .map_err(|_| DriveFault::Denied)?;
        let lba_mid = self.bus.read_byte(LBA_MID_PORT).map_err(|_| DriveFault::Denied)?;
        let lba_high = self.bus.read_byte(LBA_HIGH_PORT).map_err(|_| DriveFault::Denied)?;
        Ok(sector_count <= 1 && lba_mid == 0 && lba_high == 0)
    }

    fn wait_ready(&mut self, require_data_request: bool) -> Result<(), DriveFault> {
        let mut slept_ticks = 0u64;
        loop {
            for _ in 0..STATUS_SPINS_BEFORE_SLEEP {
                let status = self
                    .bus
                    .read_byte(ALTERNATE_STATUS_PORT)
                    .map_err(|_| DriveFault::Denied)?;
                if status & STATUS_BUSY == 0 {
                    if status & STATUS_ERROR != 0 {
                        let error_register = self
                            .bus
                            .read_byte(ERROR_REGISTER_PORT)
                            .unwrap_or(status);
                        return Err(DriveFault::Device {
                            error_register,
                            status,
                        });
                    }
                    if !require_data_request || status & STATUS_DATA_REQUEST != 0 {
                        return Ok(());
                    }
                }
            }
            if slept_ticks >= SLEEP_TICKS_BEFORE_BUSY {
                return Err(DriveFault::Busy);
            }
            self.bus.sleep_ms(1);
            slept_ticks += 1;
        }
    }
}

fn parse_identity(block: &[u16; IDENTIFY_WORD_COUNT]) -> Identity {
    Identity {
        model: decode_identity_text(block, MODEL_FIRST_WORD, MODEL_LAST_WORD),
        firmware_rev: decode_identity_text(block, FIRMWARE_FIRST_WORD, FIRMWARE_LAST_WORD),
        sectors: decode_capacity(block),
    }
}

fn decode_identity_text(block: &[u16; IDENTIFY_WORD_COUNT], first: usize, last: usize) -> String {
    let mut text = String::new();
    for word in &block[first..=last] {
        for byte in [(word >> 8) as u8, *word as u8] {
            text.push(if (0x20..=0x7E).contains(&byte) {
                byte as char
            } else {
                ' '
            });
        }
    }
    text.trim_end().to_string()
}

fn decode_capacity(block: &[u16; IDENTIFY_WORD_COUNT]) -> u64 {
    let lba28 =
        u64::from(block[CAPACITY_LOW_WORD]) | (u64::from(block[CAPACITY_HIGH_WORD]) << 16);
    let capacity = if lba28 == 0 {
        u64::from(block[CHS_SECTORS_PER_TRACK_WORD])
            * u64::from(block[CHS_HEADS_WORD])
            * u64::from(block[CHS_CYLINDERS_WORD])
    } else {
        lba28
    };
    capacity.min(u64::from(LBA28_MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::VecDeque;
    use alloc::vec;
    use alloc::vec::Vec;

    const READY_WITH_DRQ: u8 = 0x58;
    const FAULTED: u8 = 0x41;
    const ABORT: u8 = 0x10;
    const SECTOR_ONE: u8 = 1;

    #[derive(Debug, PartialEq)]
    enum Op {
        Out(u16, u8),
        InWords(u16, Vec<u16>),
        OutWords(u16, Vec<u16>),
        Sleep(u64),
    }

    struct MockBus {
        ops: Vec<Op>,
        disk_words: VecDeque<u16>,
        command_status: u8,
        error_register: u8,
        signature_sector_count: u8,
        signature_lba_mid: u8,
        signature_lba_high: u8,
        busy_stuck: bool,
        fault_status_after_reads: usize,
        fault_status: u8,
        data_reads: usize,
        sleeps: u64,
        alternate_reads: usize,
        deny_reads: bool,
        deny_writes: bool,
    }

    impl MockBus {
        fn ready() -> MockBus {
            MockBus {
                ops: Vec::new(),
                disk_words: VecDeque::new(),
                command_status: READY_WITH_DRQ,
                error_register: 0,
                signature_sector_count: 1,
                signature_lba_mid: 0,
                signature_lba_high: 0,
                busy_stuck: false,
                fault_status_after_reads: usize::MAX,
                fault_status: READY_WITH_DRQ,
                data_reads: 0,
                sleeps: 0,
                alternate_reads: 0,
                deny_reads: false,
                deny_writes: false,
            }
        }

        fn alternate_status(&self) -> u8 {
            if self.busy_stuck {
                STATUS_BUSY
            } else if self.data_reads >= self.fault_status_after_reads {
                self.fault_status
            } else {
                READY_WITH_DRQ
            }
        }
    }

    impl Bus for MockBus {
        fn read_byte(&mut self, port: u16) -> Result<u8, PortError> {
            if self.deny_reads {
                return Err(PortError::Denied);
            }
            let value = match port {
                ALTERNATE_STATUS_PORT => {
                    self.alternate_reads += 1;
                    self.alternate_status()
                }
                COMMAND_PORT => self.command_status,
                ERROR_REGISTER_PORT => self.error_register,
                SECTOR_COUNT_PORT => self.signature_sector_count,
                LBA_MID_PORT => self.signature_lba_mid,
                LBA_HIGH_PORT => self.signature_lba_high,
                other => panic!("no scripted read for port {other:#06x}"),
            };
            Ok(value)
        }

        fn write_byte(&mut self, port: u16, value: u8) -> Result<(), PortError> {
            if self.deny_writes {
                return Err(PortError::Denied);
            }
            self.ops.push(Op::Out(port, value));
            Ok(())
        }

        fn read_words(&mut self, port: u16, destination: &mut [u16]) -> Result<(), PortError> {
            if self.deny_reads {
                return Err(PortError::Denied);
            }
            if port == DATA_PORT {
                self.data_reads += 1;
            }
            let mut words = Vec::with_capacity(destination.len());
            for slot in destination.iter_mut() {
                let word = self
                    .disk_words
                    .pop_front()
                    .expect("mock disk ran out of words");
                *slot = word;
                words.push(word);
            }
            self.ops.push(Op::InWords(port, words));
            Ok(())
        }

        fn write_words(&mut self, port: u16, source: &[u16]) -> Result<(), PortError> {
            if self.deny_writes {
                return Err(PortError::Denied);
            }
            self.ops.push(Op::OutWords(port, source.to_vec()));
            Ok(())
        }

        fn sleep_ms(&mut self, millis: u64) {
            self.sleeps += millis;
            self.ops.push(Op::Sleep(millis));
        }
    }

    fn drive_on(bus: MockBus) -> AtaDrive<MockBus> {
        AtaDrive::new(bus, 0)
    }

    fn probed_drive() -> AtaDrive<MockBus> {
        let mut bus = MockBus::ready();
        bus.disk_words.extend(identify_block_words());
        let mut drive = drive_on(bus);
        drive.probe().unwrap();
        drive.bus.ops.clear();
        drive.bus.data_reads = 0;
        drive
    }

    fn put_text(block: &mut [u16; IDENTIFY_WORD_COUNT], first_word: usize, text: &str) {
        let bytes = text.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let first = bytes[index];
            let second = if index + 1 < bytes.len() {
                bytes[index + 1]
            } else {
                b' '
            };
            block[first_word + index / 2] = ((first as u16) << 8) | (second as u16);
            index += 2;
        }
    }

    fn identify_block_words() -> Vec<u16> {
        let mut block = [0u16; IDENTIFY_WORD_COUNT];
        block[0] = 0x0040;
        put_text(&mut block, FIRMWARE_FIRST_WORD, "1.0");
        put_text(&mut block, MODEL_FIRST_WORD, "QEMU HARDDISK");
        block[CAPACITY_LOW_WORD] = 7106;
        block[CAPACITY_HIGH_WORD] = 0;
        block.to_vec()
    }

    fn sector_words(sector: usize) -> Vec<u16> {
        (0..WORDS_PER_SECTOR).map(|word| (0x0100 + word + sector * 0x2000) as u16).collect()
    }

    fn sector_pattern(sector: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(SECTOR_BYTES);
        for word in sector_words(sector) {
            bytes.push(word as u8);
            bytes.push((word >> 8) as u8);
        }
        bytes
    }

    #[test]
    fn absent_drive_floating_bus_is_not_found() {
        let mut bus = MockBus::ready();
        bus.command_status = FLOATING_BUS_BYTE;
        let mut drive = drive_on(bus);
        assert_eq!(drive.probe(), Err(DriveFault::NotFound));
    }

    #[test]
    fn probe_decodes_swapped_identify_strings_and_capacity() {
        let mut bus = MockBus::ready();
        bus.disk_words.extend(identify_block_words());
        let mut drive = drive_on(bus);
        let identity = drive.probe().unwrap();
        assert_eq!(identity.model, "QEMU HARDDISK");
        assert_eq!(identity.firmware_rev, "1.0");
        assert_eq!(identity.sectors, 7106);
        assert_eq!(drive.identity(), Some(&identity));
    }

    #[test]
    fn probe_falls_back_to_chs_geometry_without_lba_capacity() {
        let mut bus = MockBus::ready();
        let mut block = [0u16; IDENTIFY_WORD_COUNT];
        put_text(&mut block, MODEL_FIRST_WORD, "OLD DISK");
        block[CHS_SECTORS_PER_TRACK_WORD] = 63;
        block[CHS_HEADS_WORD] = 16;
        block[CHS_CYLINDERS_WORD] = 16;
        bus.disk_words.extend(block.to_vec());
        let mut drive = drive_on(bus);
        let identity = drive.probe().unwrap();
        assert_eq!(identity.sectors, 63 * 16 * 16);
    }

    #[test]
    fn probe_rejects_non_disk_signature() {
        let mut bus = MockBus::ready();
        bus.signature_sector_count = 0x02;
        bus.signature_lba_mid = 0x14;
        bus.signature_lba_high = 0xEB;
        let mut drive = drive_on(bus);
        assert_eq!(drive.probe(), Err(DriveFault::NotFound));
    }

    #[test]
    fn probe_rejects_floating_signature_registers() {
        let mut bus = MockBus::ready();
        bus.signature_sector_count = 0xFF;
        bus.signature_lba_mid = 0xFF;
        bus.signature_lba_high = 0xFF;
        let mut drive = drive_on(bus);
        assert_eq!(drive.probe(), Err(DriveFault::NotFound));
    }

    #[test]
    fn denied_ports_surface_as_denied_fault() {
        let mut bus = MockBus::ready();
        bus.deny_reads = true;
        let mut drive = drive_on(bus);
        assert_eq!(drive.probe(), Err(DriveFault::Denied));

        let mut bus = MockBus::ready();
        bus.deny_writes = true;
        let mut drive = drive_on(bus);
        assert_eq!(drive.probe(), Err(DriveFault::Denied));
    }

    #[test]
    fn read_moves_two_sectors_with_per_sector_data_request() {
        let mut drive = probed_drive();
        drive.bus.disk_words.extend(sector_words(0));
        drive.bus.disk_words.extend(sector_words(1));

        let mut buffer = vec![0u8; 2 * SECTOR_BYTES];
        drive.read_sectors(0, 2, &mut buffer).unwrap();

        assert_eq!(&buffer[..SECTOR_BYTES], &sector_pattern(0)[..]);
        assert_eq!(&buffer[SECTOR_BYTES..], &sector_pattern(1)[..]);
        let block_reads = drive
            .bus
            .ops
            .iter()
            .filter(|op| matches!(op, Op::InWords(DATA_PORT, _)))
            .count();
        assert_eq!(block_reads, 2);
    }

    #[test]
    fn read_reports_device_fault_with_snapshot_after_first_sector() {
        let mut drive = probed_drive();
        drive.bus.disk_words.extend(sector_words(0));
        drive.bus.fault_status_after_reads = 1;
        drive.bus.fault_status = FAULTED;
        drive.bus.error_register = ABORT;

        let mut buffer = vec![0u8; 2 * SECTOR_BYTES];
        assert_eq!(
            drive.read_sectors(0, 2, &mut buffer),
            Err(DriveFault::Device {
                error_register: ABORT,
                status: FAULTED,
            })
        );
        assert_eq!(&buffer[..SECTOR_BYTES], &sector_pattern(0)[..]);
    }

    #[test]
    fn busy_status_forever_becomes_busy_within_budget() {
        let mut drive = probed_drive();
        drive.bus.busy_stuck = true;
        drive.bus.alternate_reads = 0;
        let mut buffer = vec![0u8; SECTOR_BYTES];
        assert_eq!(drive.read_sectors(0, SECTOR_ONE, &mut buffer), Err(DriveFault::Busy));
        assert_eq!(drive.bus.sleeps, SLEEP_TICKS_BEFORE_BUSY);
        assert!(
            drive.bus.alternate_reads
                <= (SLEEP_TICKS_BEFORE_BUSY as usize + 1) * STATUS_SPINS_BEFORE_SLEEP + 16
        );
    }

    #[test]
    fn read_beyond_identified_capacity_is_bad_address() {
        let mut drive = probed_drive();
        let mut buffer = vec![0u8; SECTOR_BYTES];
        assert_eq!(
            drive.read_sectors(7106, SECTOR_ONE, &mut buffer),
            Err(DriveFault::BadAddress)
        );
        assert_eq!(drive.bus.ops, vec![]);
    }

    #[test]
    fn write_issues_exact_task_file_sequence() {
        let mut drive = probed_drive();
        let payload = sector_pattern(3);
        drive.write_sectors(0x1020, &payload).unwrap();

        assert_eq!(
            drive.bus.ops,
            vec![
                Op::Out(SECTOR_COUNT_PORT, SECTOR_ONE),
                Op::Out(LBA_LOW_PORT, 0x20),
                Op::Out(LBA_MID_PORT, 0x10),
                Op::Out(LBA_HIGH_PORT, 0x00),
                Op::Out(DRIVE_HEAD_PORT, 0xE0),
                Op::Out(COMMAND_PORT, WRITE_SECTORS_COMMAND),
                Op::OutWords(DATA_PORT, sector_words(3)),
            ]
        );
    }

    #[test]
    fn write_beyond_identified_capacity_is_bad_address() {
        let mut drive = probed_drive();
        let payload = vec![0u8; 2 * SECTOR_BYTES];
        assert_eq!(
            drive.write_sectors(7105, &payload),
            Err(DriveFault::BadAddress)
        );
        assert_eq!(drive.bus.ops, vec![]);
    }

    #[test]
    fn write_without_probed_identity_is_not_found() {
        let bus = MockBus::ready();
        let mut drive = drive_on(bus);
        assert_eq!(
            drive.write_sectors(0, &sector_pattern(0)),
            Err(DriveFault::NotFound)
        );
    }

    #[test]
    fn flush_sends_command_and_waits() {
        let mut drive = probed_drive();
        drive.flush().unwrap();
        assert_eq!(drive.bus.ops, vec![Op::Out(COMMAND_PORT, FLUSH_CACHE_COMMAND)]);
    }

    #[test]
    fn flush_without_probed_identity_is_not_found() {
        let bus = MockBus::ready();
        let mut drive = drive_on(bus);
        assert_eq!(drive.flush(), Err(DriveFault::NotFound));
    }
}
