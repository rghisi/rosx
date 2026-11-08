use crate::provider::{Provider, ProviderGeometry, ATA_SERVICE, ATA_SOURCE};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use block_protocol::manager::wire::{DeviceInfo, DeviceKind, DeviceState, DESCRIPTION_BYTES};
use block_protocol::wire::BlockError;
use core::fmt;
use rxpart::{parse_mbr, Exposure, Table, TableState, SECTOR_BYTES};

const MAX_DRIVES: usize = 8;

pub fn provider_source(service: &str) -> Option<&'static str> {
    match service {
        ATA_SERVICE => Some(ATA_SOURCE),
        _ => None,
    }
}

struct RegisteredProvider {
    service: String,
    provider: Box<dyn Provider>,
}

pub struct DeviceEntry {
    pub name: String,
    pub source: String,
    pub description: String,
    pub kind: DeviceKind,
    pub state: DeviceState,
    provider_slot: usize,
    provider_device: u32,
    base: u64,
    len: u64,
    block_size: u32,
}

impl DeviceEntry {
    fn info(&self) -> DeviceInfo {
        DeviceInfo {
            name: self.name.clone(),
            source: self.source.clone(),
            description: self.description.clone(),
            block_count: self.len,
            block_size: self.block_size,
            kind: self.kind,
            state: self.state,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveVerdict {
    pub drive: String,
    pub exposed: Vec<String>,
    pub whole_state: Option<DeviceState>,
    pub refused_containers: Vec<u8>,
}

impl fmt::Display for DriveVerdict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.whole_state {
            Some(state) => write!(formatter, "{} whole {}", self.drive, state.label())?,
            None => {
                write!(formatter, "{} hidden, slices", self.drive)?;
                for name in &self.exposed {
                    write!(formatter, " {name}")?;
                }
            }
        }
        if !self.refused_containers.is_empty() {
            write!(formatter, ", refused containers")?;
            for type_id in &self.refused_containers {
                write!(formatter, " 0x{type_id:02X}")?;
            }
        }
        Ok(())
    }
}

pub struct DeviceTable {
    providers: Vec<RegisteredProvider>,
    entries: Vec<DeviceEntry>,
}

impl DeviceTable {
    pub fn new() -> DeviceTable {
        DeviceTable { providers: Vec::new(), entries: Vec::new() }
    }

    pub fn register(
        &mut self,
        service: &str,
        provider: Box<dyn Provider>,
    ) -> Result<Vec<DriveVerdict>, BlockError> {
        let Some(source) = provider_source(service) else {
            return Err(BlockError::Unsupported);
        };
        let slot = match self.providers.iter().position(|open| open.service == service) {
            Some(slot) => {
                self.providers[slot].provider = provider;
                slot
            }
            None => {
                self.providers.push(RegisteredProvider { service: String::from(service), provider });
                self.providers.len() - 1
            }
        };
        self.entries.retain(|entry| entry.provider_slot != slot);
        let mut verdicts = Vec::new();
        for drive in 0..MAX_DRIVES as u32 {
            let geometry = match self.providers[slot].provider.probe(drive) {
                Ok(geometry) => geometry,
                Err(BlockError::NotFound) => break,
                Err(error) => return Err(error),
            };
            let sector = read_master_boot_record(&mut *self.providers[slot].provider, drive, geometry.block_size)?;
            let (entries, verdict) = entries_of(source, slot, drive, &geometry, &sector);
            self.entries.extend(entries);
            verdicts.push(verdict);
        }
        Ok(verdicts)
    }

    pub fn list(&self) -> Vec<DeviceInfo> {
        self.entries.iter().map(DeviceEntry::info).collect()
    }

    pub fn probe(&self, name: &str) -> Result<DeviceInfo, BlockError> {
        self.lookup(name).map(DeviceEntry::info)
    }

    pub fn read(&mut self, name: &str, block: u64, count: u16) -> Result<Vec<u8>, BlockError> {
        let (slot, drive, device_block) = self.resolve(name, block, count as u64)?;
        self.providers[slot].provider.read(drive, device_block, count)
    }

    pub fn write(&mut self, name: &str, block: u64, count: u16, data: &[u8]) -> Result<(), BlockError> {
        let (slot, drive, device_block) = self.resolve(name, block, count as u64)?;
        self.providers[slot].provider.write(drive, device_block, count, data)
    }

    pub fn flush(&mut self, name: &str) -> Result<(), BlockError> {
        let entry = self.lookup(name)?;
        let (slot, drive) = (entry.provider_slot, entry.provider_device);
        self.providers[slot].provider.flush(drive)
    }

    fn lookup(&self, name: &str) -> Result<&DeviceEntry, BlockError> {
        self.entries.iter().find(|entry| entry.name == name).ok_or(BlockError::NotFound)
    }

    fn resolve(&self, name: &str, block: u64, count: u64) -> Result<(usize, u32, u64), BlockError> {
        let entry = self.lookup(name)?;
        let last = block.checked_add(count).ok_or(BlockError::BadAddress)?;
        if last > entry.len {
            return Err(BlockError::BadAddress);
        }
        let device_block = entry.base.checked_add(block).ok_or(BlockError::BadAddress)?;
        Ok((entry.provider_slot, entry.provider_device, device_block))
    }
}

fn read_master_boot_record(
    provider: &mut dyn Provider,
    drive: u32,
    block_size: u32,
) -> Result<[u8; SECTOR_BYTES], BlockError> {
    let blocks = (SECTOR_BYTES / block_size.max(1) as usize).max(1) as u16;
    let payload = provider.read(drive, 0, blocks)?;
    let record: &[u8; SECTOR_BYTES] = payload
        .get(..SECTOR_BYTES)
        .ok_or(BlockError::Device)?
        .try_into()
        .map_err(|_| BlockError::Device)?;
    Ok(*record)
}

fn entries_of(
    source: &str,
    provider_slot: usize,
    drive: u32,
    geometry: &ProviderGeometry,
    sector: &[u8; SECTOR_BYTES],
) -> (Vec<DeviceEntry>, DriveVerdict) {
    let drive_name = format!("{source}d{drive}");
    let table = parse_mbr(sector, geometry.block_count);
    let refused_containers = refused_container_types(&table);
    match rxpart::exposure(&table) {
        Exposure::Whole { table: table_state } => {
            let state = match table_state {
                TableState::Corrupt => DeviceState::Corrupt,
                _ => DeviceState::Clean,
            };
            let description = match table_state {
                TableState::Valid => {
                    whole_description(&geometry.description, &refused_containers)
                }
                _ => fit_description(&geometry.description),
            };
            let entry = DeviceEntry {
                name: drive_name.clone(),
                source: String::from(source),
                description,
                kind: DeviceKind::Disk,
                state,
                provider_slot,
                provider_device: drive,
                base: 0,
                len: geometry.block_count,
                block_size: geometry.block_size,
            };
            let exposed = vec![drive_name.clone()];
            (
                vec![entry],
                DriveVerdict { drive: drive_name, exposed, whole_state: Some(state), refused_containers },
            )
        }
        Exposure::Partitions(slices) => {
            let entries: Vec<DeviceEntry> = slices
                .into_iter()
                .map(|slice| DeviceEntry {
                    name: format!("{drive_name}p{}", slice.slot),
                    source: String::from(source),
                    description: slice_description(&drive_name, slice.type_id, &geometry.description),
                    kind: DeviceKind::Partition,
                    state: DeviceState::Clean,
                    provider_slot,
                    provider_device: drive,
                    base: u64::from(slice.start_lba),
                    len: u64::from(slice.sector_count),
                    block_size: geometry.block_size,
                })
                .collect();
            let exposed = entries.iter().map(|entry| entry.name.clone()).collect();
            (
                entries,
                DriveVerdict { drive: drive_name, exposed, whole_state: None, refused_containers },
            )
        }
    }
}

fn refused_container_types(table: &Table) -> Vec<u8> {
    let mut refused: Vec<u8> = Vec::new();
    for entry in table.entries.iter().flatten() {
        if !rxpart::is_supported(entry.type_id) && !refused.contains(&entry.type_id) {
            refused.push(entry.type_id);
        }
    }
    refused
}

fn whole_description(model: &str, refused_containers: &[u8]) -> String {
    if refused_containers.is_empty() {
        return fit_description(model);
    }
    let mut note = String::from(" (containers refused:");
    for type_id in refused_containers {
        note.push_str(&format!(" 0x{type_id:02X}"));
    }
    note.push(')');
    let mut description = take_prefix(model, DESCRIPTION_BYTES.saturating_sub(note.len()));
    description.push_str(&note);
    description
}

fn slice_description(drive_name: &str, type_id: u8, model: &str) -> String {
    let verdict = format!("{drive_name} type {type_id:02X}");
    let with_model = format!("{verdict} {model}");
    if with_model.len() <= DESCRIPTION_BYTES {
        with_model
    } else {
        verdict
    }
}

fn fit_description(text: &str) -> String {
    take_prefix(text, DESCRIPTION_BYTES)
}

fn take_prefix(text: &str, budget: usize) -> String {
    let mut prefix = String::new();
    for character in text.chars() {
        if prefix.len() + character.len_utf8() > budget {
            break;
        }
        prefix.push(character);
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use alloc::vec;
    use core::cell::RefCell;
    use rxpart::{ENTRY_BYTES, TABLE_OFFSET};

    #[derive(Default)]
    struct CallLog {
        reads: Vec<(u32, u64, u16)>,
        writes: Vec<(u32, u64, u16, usize)>,
        flushes: Vec<u32>,
    }

    struct FakeProvider {
        geometries: Vec<Option<(u64, u32)>>,
        sectors: Vec<Option<[u8; SECTOR_BYTES]>>,
        models: Vec<Option<String>>,
        probe_fault: Option<(u32, BlockError)>,
        io_faulting: Rc<RefCell<bool>>,
        log: Rc<RefCell<CallLog>>,
    }

    impl FakeProvider {
        fn drives(geometries: Vec<Option<(u64, u32)>>) -> FakeProvider {
            FakeProvider {
                sectors: vec![None; geometries.len()],
                geometries,
                models: Vec::new(),
                probe_fault: None,
                io_faulting: Rc::new(RefCell::new(false)),
                log: Rc::new(RefCell::new(CallLog::default())),
            }
        }

        fn scripted(geometries: Vec<Option<(u64, u32)>>, log: Rc<RefCell<CallLog>>) -> FakeProvider {
            let mut provider = FakeProvider::drives(geometries);
            provider.log = log;
            provider
        }

        fn with_sector(mut self, drive: usize, sector: [u8; SECTOR_BYTES]) -> FakeProvider {
            self.sectors.resize(self.sectors.len().max(drive + 1), None);
            self.sectors[drive] = Some(sector);
            self
        }

        fn with_model(mut self, drive: usize, model: &str) -> FakeProvider {
            self.models.resize(self.models.len().max(drive + 1), None);
            self.models[drive] = Some(String::from(model));
            self
        }

        fn with_probe_fault(mut self, drive: u32, error: BlockError) -> FakeProvider {
            self.probe_fault = Some((drive, error));
            self
        }

        fn io_switch(&self) -> Rc<RefCell<bool>> {
            self.io_faulting.clone()
        }
    }

    impl Provider for FakeProvider {
        fn probe(&mut self, drive: u32) -> Result<ProviderGeometry, BlockError> {
            if let Some((fault_drive, error)) = self.probe_fault {
                if fault_drive == drive {
                    return Err(error);
                }
            }
            match self.geometries.get(drive as usize) {
                Some(Some((blocks, size))) => Ok(ProviderGeometry {
                    block_count: *blocks,
                    block_size: *size,
                    description: self
                        .models
                        .get(drive as usize)
                        .cloned()
                        .flatten()
                        .unwrap_or_else(|| format!("drive {drive}")),
                }),
                _ => Err(BlockError::NotFound),
            }
        }

        fn read(&mut self, drive: u32, block: u64, count: u16) -> Result<Vec<u8>, BlockError> {
            self.log.borrow_mut().reads.push((drive, block, count));
            if *self.io_faulting.borrow() {
                return Err(BlockError::Device);
            }
            let scripted = self.sectors.get(drive as usize).cloned().flatten();
            Ok((0..count as u64)
                .flat_map(|offset| match (block + offset, scripted) {
                    (0, Some(sector)) => sector.to_vec(),
                    _ => vec![0u8; SECTOR_BYTES],
                })
                .collect())
        }

        fn write(&mut self, drive: u32, block: u64, count: u16, data: &[u8]) -> Result<(), BlockError> {
            self.log.borrow_mut().writes.push((drive, block, count, data.len()));
            if *self.io_faulting.borrow() {
                return Err(BlockError::Device);
            }
            Ok(())
        }

        fn flush(&mut self, drive: u32) -> Result<(), BlockError> {
            self.log.borrow_mut().flushes.push(drive);
            if *self.io_faulting.borrow() {
                return Err(BlockError::Device);
            }
            Ok(())
        }
    }

    fn master_boot_record(entries: &[(usize, u8, u32, u32)]) -> [u8; SECTOR_BYTES] {
        let mut sector = [0u8; SECTOR_BYTES];
        sector[SECTOR_BYTES - 2] = 0x55;
        sector[SECTOR_BYTES - 1] = 0xAA;
        for (slot, type_id, start_lba, sector_count) in entries {
            let entry = &mut sector[TABLE_OFFSET + slot * ENTRY_BYTES..][..ENTRY_BYTES];
            entry[4] = *type_id;
            entry[8..12].copy_from_slice(&start_lba.to_le_bytes());
            entry[12..16].copy_from_slice(&sector_count.to_le_bytes());
        }
        sector
    }

    fn table_with(drives: Vec<Option<(u64, u32)>>) -> DeviceTable {
        let mut table = DeviceTable::new();
        table.register(ATA_SERVICE, Box::new(FakeProvider::drives(drives))).expect("register must succeed");
        table
    }

    fn table_of(provider: FakeProvider) -> DeviceTable {
        let mut table = DeviceTable::new();
        table.register(ATA_SERVICE, Box::new(provider)).expect("register must succeed");
        table
    }

    #[test]
    fn empty_table_lists_nothing_and_looks_up_nothing() {
        let table = DeviceTable::new();
        assert!(table.list().is_empty());
        assert_eq!(table.probe("ata0d0"), Err(BlockError::NotFound));
    }

    #[test]
    fn register_discovers_contiguous_drives_until_not_found() {
        let table = table_with(vec![Some((1024, 512)), Some((2048, 512))]);
        let names: Vec<String> = table.list().iter().map(|d| d.name.clone()).collect();
        assert_eq!(names, vec!["ata0d0", "ata0d1"]);
    }

    #[test]
    fn register_stops_at_the_first_absent_drive() {
        let table = table_with(vec![Some((1024, 512)), None, Some((4096, 512))]);
        let names: Vec<String> = table.list().iter().map(|d| d.name.clone()).collect();
        assert_eq!(names, vec!["ata0d0"], "a gap in the drive sequence ends discovery");
    }

    #[test]
    fn register_caps_discovery_at_eight_drives() {
        let drives = vec![Some((64, 512)); 12];
        let table = table_with(drives);
        assert_eq!(table.list().len(), 8);
    }

    #[test]
    fn register_records_honest_geometry_and_clean_disk_shape() {
        let table = table_with(vec![Some((102400, 512))]);
        let device = table.probe("ata0d0").unwrap();
        assert_eq!(device.source, "ata0");
        assert_eq!(device.block_count, 102400);
        assert_eq!(device.block_size, 512);
        assert_eq!(device.kind, DeviceKind::Disk);
        assert_eq!(device.state, DeviceState::Clean);
        assert_eq!(device.description, "drive 0");
    }

    #[test]
    fn register_of_an_unknown_service_is_unsupported_and_touches_nothing() {
        let mut table = DeviceTable::new();
        let provider: Box<dyn Provider> = Box::new(FakeProvider::drives(vec![Some((10, 512))]));
        assert_eq!(table.register("SCSI", provider), Err(BlockError::Unsupported));
        assert!(table.list().is_empty());
    }

    #[test]
    fn re_register_refreshes_the_namespace_for_the_same_service() {
        let mut table = DeviceTable::new();
        table.register(ATA_SERVICE, Box::new(FakeProvider::drives(vec![Some((100, 512)), Some((200, 512))]))).unwrap();
        assert_eq!(table.list().len(), 2);
        table.register(ATA_SERVICE, Box::new(FakeProvider::drives(vec![Some((999, 512))]))).unwrap();
        let devices = table.list();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "ata0d0");
        assert_eq!(devices[0].block_count, 999);
    }

    #[test]
    fn a_refused_register_leaves_the_live_namespace_intact() {
        let mut table = table_with(vec![Some((100, 512)), Some((200, 512))]);
        let intruder: Box<dyn Provider> = Box::new(FakeProvider::drives(vec![Some((9, 512))]));
        assert_eq!(table.register("SCSI", intruder), Err(BlockError::Unsupported));
        assert_eq!(table.list().len(), 2);
        assert_eq!(table.probe("ata0d1").unwrap().block_count, 200);
    }

    #[test]
    fn a_probe_fault_other_than_not_found_surfaces_honestly() {
        let mut table = DeviceTable::new();
        let provider = Box::new(FakeProvider::drives(vec![Some((10, 512))]).with_probe_fault(0, BlockError::Device));
        assert_eq!(table.register(ATA_SERVICE, provider), Err(BlockError::Device));
        assert!(table.list().is_empty());
    }

    #[test]
    fn probe_reads_the_table_by_name() {
        let table = table_with(vec![Some((100, 512)), Some((200, 1024))]);
        assert_eq!(table.probe("ata0d1").unwrap().block_size, 1024);
        assert_eq!(table.probe("ata0d2"), Err(BlockError::NotFound));
        assert_eq!(table.probe("ata1d0"), Err(BlockError::NotFound));
    }

    #[test]
    fn read_translates_with_the_base_offset_and_bounds_against_len() {
        let mut table = table_with(vec![Some((100, 512))]);
        assert!(table.read("ata0d0", 90, 10).is_ok());
        assert_eq!(table.read("ata0d0", 95, 6), Err(BlockError::BadAddress), "one block past the end");
        assert_eq!(table.read("ata0d0", 100, 1), Err(BlockError::BadAddress), "start at len");
        assert_eq!(table.read("ata0d0", u64::MAX, 1), Err(BlockError::BadAddress), "count overflow");
    }

    #[test]
    fn read_forwards_the_provider_slot_and_device_block() {
        let log = Rc::new(RefCell::new(CallLog::default()));
        let mut table = DeviceTable::new();
        table
            .register(ATA_SERVICE, Box::new(FakeProvider::scripted(vec![Some((100, 512)), Some((200, 512))], log.clone())))
            .unwrap();
        assert_eq!(
            log.borrow().reads,
            vec![(0u32, 0u64, 1u16), (1u32, 0u64, 1u16)],
            "registration reads sector 0 of every discovered drive"
        );
        table.read("ata0d1", 5, 3).unwrap();
        assert_eq!(
            *log.borrow().reads.last().unwrap(),
            (1u32, 5u64, 3u16),
            "drive 1, base 0, block 5"
        );
    }

    #[test]
    fn read_of_an_unknown_name_is_not_found_before_touching_a_provider() {
        let mut table = table_with(vec![Some((100, 512))]);
        assert_eq!(table.read("ghost", 0, 1), Err(BlockError::NotFound));
    }

    #[test]
    fn write_bounds_then_forwards() {
        let log = Rc::new(RefCell::new(CallLog::default()));
        let mut table = DeviceTable::new();
        table
            .register(ATA_SERVICE, Box::new(FakeProvider::scripted(vec![Some((100, 512))], log.clone())))
            .unwrap();
        let payload = vec![7u8; 512];
        assert!(table.write("ata0d0", 99, 1, &payload).is_ok());
        assert_eq!(table.write("ata0d0", 99, 2, &payload), Err(BlockError::BadAddress));
        assert_eq!(table.write("ghost", 0, 1, &payload), Err(BlockError::NotFound));
        assert_eq!(log.borrow().writes, vec![(0u32, 99u64, 1u16, 512)], "only the bounded write reached the provider");
    }

    #[test]
    fn flush_forwards_to_the_provider_drive() {
        let log = Rc::new(RefCell::new(CallLog::default()));
        let mut table = DeviceTable::new();
        table
            .register(ATA_SERVICE, Box::new(FakeProvider::scripted(vec![Some((100, 512)), Some((200, 512))], log.clone())))
            .unwrap();
        assert!(table.flush("ata0d1").is_ok());
        assert_eq!(table.flush("ghost"), Err(BlockError::NotFound));
        assert_eq!(log.borrow().flushes, vec![1u32]);
    }

    #[test]
    fn provider_io_faults_surface_from_forwarded_operations() {
        let mut table = DeviceTable::new();
        let provider = FakeProvider::drives(vec![Some((100, 512))]);
        let faulting = provider.io_switch();
        table.register(ATA_SERVICE, Box::new(provider)).unwrap();
        *faulting.borrow_mut() = true;
        assert_eq!(table.read("ata0d0", 0, 1), Err(BlockError::Device));
        assert_eq!(table.write("ata0d0", 0, 1, &[0u8; 512]), Err(BlockError::Device));
        assert_eq!(table.flush("ata0d0"), Err(BlockError::Device));
    }

    #[test]
    fn provider_source_maps_only_known_services() {
        assert_eq!(provider_source("ATA"), Some("ata0"));
        assert_eq!(provider_source("SCSI"), None);
        assert_eq!(provider_source(""), None);
    }

    #[test]
    fn a_drive_without_signature_is_listed_whole_and_reported() {
        let mut table = DeviceTable::new();
        let verdicts = table
            .register(ATA_SERVICE, Box::new(FakeProvider::drives(vec![Some((1024, 512))])))
            .unwrap();
        assert_eq!(
            verdicts,
            vec![DriveVerdict {
                drive: String::from("ata0d0"),
                exposed: vec![String::from("ata0d0")],
                whole_state: Some(DeviceState::Clean),
                refused_containers: Vec::new(),
            }]
        );
        let device = table.probe("ata0d0").unwrap();
        assert_eq!(device.kind, DeviceKind::Disk);
        assert_eq!(device.state, DeviceState::Clean);
        assert_eq!(device.block_count, 1024);
    }

    #[test]
    fn a_signed_empty_table_stays_whole_and_clean() {
        let table = table_of(FakeProvider::drives(vec![Some((1024, 512))]).with_sector(0, master_boot_record(&[])));
        let device = table.probe("ata0d0").unwrap();
        assert_eq!(device.kind, DeviceKind::Disk);
        assert_eq!(device.state, DeviceState::Clean);
    }

    #[test]
    fn a_corrupt_table_exposes_the_drive_honestly() {
        let sector = master_boot_record(&[(0, 0x83, u32::MAX, u32::MAX)]);
        let table = table_of(FakeProvider::drives(vec![Some((1024, 512))]).with_sector(0, sector));
        let device = table.probe("ata0d0").unwrap();
        assert_eq!(device.kind, DeviceKind::Disk);
        assert_eq!(device.state, DeviceState::Corrupt);
        assert_eq!(device.block_count, 1024);
        assert_eq!(device.description, "drive 0");
    }

    #[test]
    fn a_valid_table_hides_the_drive_and_exposes_slices_at_true_slots() {
        let sector = master_boot_record(&[(0, 0x83, 2048, 102_400), (3, 0x82, 104_448, 100_352)]);
        let table = table_of(FakeProvider::drives(vec![Some((204_800, 512))]).with_sector(0, sector));
        let names: Vec<String> = table.list().iter().map(|device| device.name.clone()).collect();
        assert_eq!(names, ["ata0d0p0", "ata0d0p3"], "holes skip p1 and p2");
        assert_eq!(table.probe("ata0d0"), Err(BlockError::NotFound), "the table wins over the drive");
        assert_eq!(table.probe("ata0d0p1"), Err(BlockError::NotFound));
        let slice = table.probe("ata0d0p3").unwrap();
        assert_eq!(slice.kind, DeviceKind::Partition);
        assert_eq!(slice.state, DeviceState::Clean);
        assert_eq!(slice.block_count, 100_352);
        assert_eq!(slice.description, "ata0d0 type 82 drive 0");
    }

    #[test]
    fn slice_reads_forward_at_the_slice_base() {
        let log = Rc::new(RefCell::new(CallLog::default()));
        let sector = master_boot_record(&[(0, 0x83, 2048, 102_400), (3, 0x82, 104_448, 100_352)]);
        let provider = FakeProvider::scripted(vec![Some((204_800, 512))], log.clone()).with_sector(0, sector);
        let mut table = table_of(provider);
        let reads_after_register = log.borrow().reads.len();
        assert_eq!(table.read("ata0d0p3", 0, 1), Ok(vec![0u8; SECTOR_BYTES]));
        assert!(table.read("ata0d0p3", 100_351, 1).is_ok());
        assert_eq!(table.read("ata0d0p3", 100_352, 1), Err(BlockError::BadAddress), "one block past the slice end");
        assert_eq!(
            log.borrow().reads[reads_after_register..],
            [(0u32, 104_448u64, 1u16), (0u32, 204_799u64, 1u16)],
            "slice block 0 lands on the partition start"
        );
    }

    #[test]
    fn a_long_model_drops_out_of_slice_descriptions() {
        let sector = master_boot_record(&[(0, 0x83, 2048, 102_400)]);
        let provider = FakeProvider::drives(vec![Some((204_800, 512))])
            .with_sector(0, sector)
            .with_model(0, &"Q".repeat(DESCRIPTION_BYTES));
        let table = table_of(provider);
        let slice = table.probe("ata0d0p0").unwrap();
        assert_eq!(slice.description, "ata0d0 type 83", "the model leaves, the verdict stays");
    }

    #[test]
    fn a_container_only_table_exposes_the_whole_drive_with_a_note() {
        let sector = master_boot_record(&[(1, 0x05, 2048, 100_000)]);
        let mut table = DeviceTable::new();
        let verdicts = table
            .register(
                ATA_SERVICE,
                Box::new(FakeProvider::drives(vec![Some((204_800, 512))]).with_sector(0, sector)),
            )
            .unwrap();
        assert_eq!(verdicts[0].refused_containers, vec![0x05]);
        let device = table.probe("ata0d0").unwrap();
        assert_eq!(device.state, DeviceState::Clean);
        assert_eq!(device.description, "drive 0 (containers refused: 0x05)");
    }

    #[test]
    fn a_long_model_loses_bytes_before_the_refusal_note() {
        let sector = master_boot_record(&[(0, 0x05, 2048, 50_000), (2, 0x0F, 52_048, 100_000)]);
        let provider = FakeProvider::drives(vec![Some((204_800, 512))])
            .with_sector(0, sector)
            .with_model(0, &"K".repeat(DESCRIPTION_BYTES));
        let table = table_of(provider);
        let device = table.probe("ata0d0").unwrap();
        assert_eq!(device.description.len(), DESCRIPTION_BYTES);
        assert!(device.description.starts_with("KKKK"), "model keeps the room it can: {}", device.description);
        assert!(device.description.ends_with("(containers refused: 0x05 0x0F)"), "the note survives whole: {}", device.description);
    }

    #[test]
    fn mixed_containers_and_data_expose_only_the_data_slices_and_log_the_refusal() {
        let sector = master_boot_record(&[(0, 0x0F, 2048, 50_000), (2, 0x83, 52_048, 100_000)]);
        let mut table = DeviceTable::new();
        let verdicts = table
            .register(
                ATA_SERVICE,
                Box::new(FakeProvider::drives(vec![Some((204_800, 512))]).with_sector(0, sector)),
            )
            .unwrap();
        assert_eq!(
            verdicts,
            vec![DriveVerdict {
                drive: String::from("ata0d0"),
                exposed: vec![String::from("ata0d0p2")],
                whole_state: None,
                refused_containers: vec![0x0F],
            }]
        );
        let names: Vec<String> = table.list().iter().map(|device| device.name.clone()).collect();
        assert_eq!(names, ["ata0d0p2"]);
    }

    #[test]
    fn register_reports_one_verdict_per_discovered_drive() {
        let sector = master_boot_record(&[(2, 0x83, 2048, 65_536)]);
        let provider = FakeProvider::drives(vec![Some((1024, 512)), Some((204_800, 512))]).with_sector(1, sector);
        let mut table = DeviceTable::new();
        let verdicts = table.register(ATA_SERVICE, Box::new(provider)).unwrap();
        assert_eq!(
            verdicts,
            vec![
                DriveVerdict {
                    drive: String::from("ata0d0"),
                    exposed: vec![String::from("ata0d0")],
                    whole_state: Some(DeviceState::Clean),
                    refused_containers: Vec::new(),
                },
                DriveVerdict {
                    drive: String::from("ata0d1"),
                    exposed: vec![String::from("ata0d1p2")],
                    whole_state: None,
                    refused_containers: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn a_sector_zero_read_fault_fails_registration_and_exposes_nothing() {
        let mut table = DeviceTable::new();
        let provider = FakeProvider::drives(vec![Some((1024, 512))]);
        *provider.io_switch().borrow_mut() = true;
        assert_eq!(table.register(ATA_SERVICE, Box::new(provider)), Err(BlockError::Device));
        assert!(table.list().is_empty(), "a provider fault is not a device: nothing gets a fake state");
    }

    #[test]
    fn re_registering_rereads_the_partition_table() {
        let sector = master_boot_record(&[(1, 0x83, 2048, 65_536)]);
        let mut table = DeviceTable::new();
        table
            .register(ATA_SERVICE, Box::new(FakeProvider::drives(vec![Some((102_400, 512))]).with_sector(0, sector)))
            .unwrap();
        assert_eq!(table.probe("ata0d0"), Err(BlockError::NotFound));
        table
            .register(ATA_SERVICE, Box::new(FakeProvider::drives(vec![Some((102_400, 512))])))
            .unwrap();
        let names: Vec<String> = table.list().iter().map(|device| device.name.clone()).collect();
        assert_eq!(names, ["ata0d0"], "the refresh re-curates from the re-read sector 0");
    }

    #[test]
    fn verdicts_render_the_exposure_decision_for_the_register_log() {
        let whole = DriveVerdict {
            drive: String::from("ata0d0"),
            exposed: vec![String::from("ata0d0")],
            whole_state: Some(DeviceState::Clean),
            refused_containers: Vec::new(),
        };
        assert_eq!(format!("{whole}"), "ata0d0 whole clean");
        let corrupt = DriveVerdict {
            drive: String::from("ata0d2"),
            exposed: vec![String::from("ata0d2")],
            whole_state: Some(DeviceState::Corrupt),
            refused_containers: vec![0x05, 0x0F],
        };
        assert_eq!(format!("{corrupt}"), "ata0d2 whole corrupt, refused containers 0x05 0x0F");
        let hidden = DriveVerdict {
            drive: String::from("ata0d1"),
            exposed: vec![String::from("ata0d1p1"), String::from("ata0d1p2")],
            whole_state: None,
            refused_containers: Vec::new(),
        };
        assert_eq!(format!("{hidden}"), "ata0d1 hidden, slices ata0d1p1 ata0d1p2");
    }
}
