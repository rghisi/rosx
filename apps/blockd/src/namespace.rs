use crate::provider::{Provider, ProviderGeometry, ATA_SERVICE, ATA_SOURCE};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use block_protocol::manager::wire::{DeviceInfo, DeviceKind, DeviceState};
use block_protocol::wire::BlockError;

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

pub struct DeviceTable {
    providers: Vec<RegisteredProvider>,
    entries: Vec<DeviceEntry>,
}

impl DeviceTable {
    pub fn new() -> DeviceTable {
        DeviceTable { providers: Vec::new(), entries: Vec::new() }
    }

    pub fn register(&mut self, service: &str, provider: Box<dyn Provider>) -> Result<(), BlockError> {
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
        for drive in 0..MAX_DRIVES as u32 {
            let probed = self.providers[slot].provider.probe(drive);
            match probed {
                Ok(geometry) => self.entries.push(entry_of(source, slot, drive, geometry)),
                Err(BlockError::NotFound) => break,
                Err(error) => return Err(error),
            }
        }
        Ok(())
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

fn entry_of(source: &str, provider_slot: usize, drive: u32, geometry: ProviderGeometry) -> DeviceEntry {
    DeviceEntry {
        name: format!("{source}d{drive}"),
        source: String::from(source),
        description: geometry.description,
        kind: DeviceKind::Disk,
        state: DeviceState::Clean,
        provider_slot,
        provider_device: drive,
        base: 0,
        len: geometry.block_count,
        block_size: geometry.block_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use alloc::vec;
    use core::cell::RefCell;

    #[derive(Default)]
    struct CallLog {
        reads: Vec<(u32, u64, u16)>,
        writes: Vec<(u32, u64, u16, usize)>,
        flushes: Vec<u32>,
    }

    struct FakeProvider {
        geometries: Vec<Option<(u64, u32)>>,
        probe_fault: Option<(u32, BlockError)>,
        io_fault: Option<BlockError>,
        log: Rc<RefCell<CallLog>>,
    }

    impl FakeProvider {
        fn drives(geometries: Vec<Option<(u64, u32)>>) -> FakeProvider {
            FakeProvider {
                geometries,
                probe_fault: None,
                io_fault: None,
                log: Rc::new(RefCell::new(CallLog::default())),
            }
        }

        fn scripted(geometries: Vec<Option<(u64, u32)>>, log: Rc<RefCell<CallLog>>) -> FakeProvider {
            FakeProvider { geometries, probe_fault: None, io_fault: None, log }
        }

        fn with_probe_fault(mut self, drive: u32, error: BlockError) -> FakeProvider {
            self.probe_fault = Some((drive, error));
            self
        }

        fn with_io_fault(mut self, error: BlockError) -> FakeProvider {
            self.io_fault = Some(error);
            self
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
                    description: format!("drive {drive}"),
                }),
                _ => Err(BlockError::NotFound),
            }
        }

        fn read(&mut self, drive: u32, block: u64, count: u16) -> Result<Vec<u8>, BlockError> {
            self.log.borrow_mut().reads.push((drive, block, count));
            match self.io_fault {
                Some(error) => Err(error),
                None => Ok(vec![drive as u8; count as usize * 512]),
            }
        }

        fn write(&mut self, drive: u32, block: u64, count: u16, data: &[u8]) -> Result<(), BlockError> {
            self.log.borrow_mut().writes.push((drive, block, count, data.len()));
            match self.io_fault {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        fn flush(&mut self, drive: u32) -> Result<(), BlockError> {
            self.log.borrow_mut().flushes.push(drive);
            match self.io_fault {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
    }

    fn table_with(drives: Vec<Option<(u64, u32)>>) -> DeviceTable {
        let mut table = DeviceTable::new();
        table.register(ATA_SERVICE, Box::new(FakeProvider::drives(drives))).expect("register must succeed");
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
        table.read("ata0d1", 5, 3).unwrap();
        assert_eq!(log.borrow().reads, vec![(1u32, 5u64, 3u16)], "drive 1, base 0, block 5");
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
        let provider = Box::new(FakeProvider::drives(vec![Some((100, 512))]).with_io_fault(BlockError::Device));
        table.register(ATA_SERVICE, provider).unwrap();
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
}
