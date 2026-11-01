use crate::device::BlockDevice;
use crate::error::FsError;
use crate::spec::{BLOCK_BYTES, CACHE_SLOTS};
use alloc::vec::Vec;

struct Slot {
    resident: Option<u64>,
    dirty: bool,
    data: [u8; BLOCK_BYTES],
}

pub struct MetaCache {
    slots: Vec<Slot>,
}

impl MetaCache {
    pub fn new() -> MetaCache {
        MetaCache::with_slots(CACHE_SLOTS)
    }

    pub fn with_slots(count: usize) -> MetaCache {
        MetaCache {
            slots: (0..count)
                .map(|_| Slot {
                    resident: None,
                    dirty: false,
                    data: [0u8; BLOCK_BYTES],
                })
                .collect(),
        }
    }

    pub fn mutate<D: BlockDevice, R>(
        &mut self,
        dev: &mut D,
        block: u64,
        edit: impl FnOnce(&mut [u8]) -> R,
    ) -> Result<R, FsError> {
        self.ensure_loaded(dev, block)?;
        let slot = self.slot_of_mut(block);
        slot.dirty = true;
        Ok(edit(&mut slot.data))
    }

    pub fn inspect<D: BlockDevice, R>(
        &mut self,
        dev: &mut D,
        block: u64,
        read: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, FsError> {
        self.ensure_loaded(dev, block)?;
        let slot = self.slot_of_mut(block);
        Ok(read(&slot.data))
    }

    pub fn invalidate(&mut self, block: u64) {
        let index = self.index(block);
        let slot = &mut self.slots[index];
        if slot.resident == Some(block) {
            slot.resident = None;
            slot.dirty = false;
        }
    }

    pub fn flush<D: BlockDevice>(&mut self, dev: &mut D) -> Result<(), FsError> {
        for slot in self.slots.iter_mut() {
            if let (Some(block), true) = (slot.resident, slot.dirty) {
                dev.write_block(block, &slot.data)
                    .map_err(FsError::Io)?;
                slot.dirty = false;
            }
        }
        Ok(())
    }

    fn ensure_loaded<D: BlockDevice>(&mut self, dev: &mut D, block: u64) -> Result<(), FsError> {
        let index = self.index(block);
        if self.slots[index].resident == Some(block) {
            return Ok(());
        }
        let slot = &mut self.slots[index];
        if let (Some(victim), true) = (slot.resident, slot.dirty) {
            dev.write_block(victim, &slot.data).map_err(FsError::Io)?;
        }
        dev.read_block(block, &mut slot.data)
            .map_err(FsError::Io)?;
        slot.resident = Some(block);
        slot.dirty = false;
        Ok(())
    }

    fn index(&self, block: u64) -> usize {
        (block % self.slots.len() as u64) as usize
    }

    fn slot_of_mut(&mut self, block: u64) -> &mut Slot {
        let index = self.index(block);
        debug_assert_eq!(self.slots[index].resident, Some(block));
        &mut self.slots[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::MemoryDevice;
    use crate::error::IoError;
    use alloc::vec::Vec;

    struct CountingDevice {
        inner: MemoryDevice,
        reads: Vec<u64>,
        writes: Vec<u64>,
    }

    impl CountingDevice {
        fn new(blocks: u64) -> CountingDevice {
            CountingDevice {
                inner: MemoryDevice::new(blocks),
                reads: Vec::new(),
                writes: Vec::new(),
            }
        }

        fn read_count(&self) -> usize {
            self.reads.len()
        }

        fn write_count(&self) -> usize {
            self.writes.len()
        }

        fn write_log(&self) -> &[u64] {
            &self.writes
        }
    }

    impl BlockDevice for CountingDevice {
        fn block_count(&self) -> u64 {
            self.inner.block_count()
        }

        fn read_block(&mut self, block: u64, buf: &mut [u8]) -> Result<(), IoError> {
            self.reads.push(block);
            self.inner.read_block(block, buf)
        }

        fn write_block(&mut self, block: u64, buf: &[u8]) -> Result<(), IoError> {
            self.writes.push(block);
            self.inner.write_block(block, buf)
        }

        fn flush(&mut self) -> Result<(), IoError> {
            self.inner.flush()
        }
    }

    fn fill_pattern(buf: &mut [u8], seed: u8) {
        for (index, byte) in buf.iter_mut().enumerate() {
            *byte = (index % 251) as u8 ^ seed;
        }
    }

    #[test]
    fn miss_loads_from_device() {
        let mut dev = CountingDevice::new(2);
        let mut pattern = [0u8; BLOCK_BYTES];
        fill_pattern(&mut pattern, 3);
        dev.write_block(1, &pattern).unwrap();
        let mut cache = MetaCache::new();
        let seen = cache.inspect(&mut dev, 1, |data| data[100]).unwrap();
        assert_eq!(seen, pattern[100]);
        assert_eq!(dev.read_count(), 1);
    }

    #[test]
    fn hit_avoids_device_reads() {
        let mut dev = CountingDevice::new(2);
        let mut cache = MetaCache::new();
        cache.inspect(&mut dev, 1, |_| ()).unwrap();
        cache.inspect(&mut dev, 1, |_| ()).unwrap();
        cache.mutate(&mut dev, 1, |_| ()).unwrap();
        assert_eq!(dev.read_count(), 1);
    }

    #[test]
    fn mutate_does_not_write_until_flush() {
        let mut dev = CountingDevice::new(1);
        let mut cache = MetaCache::new();
        cache
            .mutate(&mut dev, 0, |data| data[7] = 0xAB)
            .unwrap();
        assert_eq!(dev.write_count(), 0);
        cache.flush(&mut dev).unwrap();
        assert_eq!(dev.write_log(), &[0]);
        let mut block = [0u8; BLOCK_BYTES];
        dev.read_block(0, &mut block).unwrap();
        assert_eq!(block[7], 0xAB);
    }

    #[test]
    fn flush_skips_clean_and_empty_slots() {
        let mut dev = CountingDevice::new(4);
        let mut cache = MetaCache::new();
        cache.inspect(&mut dev, 0, |_| ()).unwrap();
        cache.mutate(&mut dev, 1, |d| d[0] = 1).unwrap();
        cache.flush(&mut dev).unwrap();
        assert_eq!(dev.write_log(), &[1]);
        cache.flush(&mut dev).unwrap();
        assert_eq!(dev.write_log(), &[1]);
    }

    #[test]
    fn dirty_eviction_writes_back_the_victim() {
        let mut dev = CountingDevice::new(200);
        let mut cache = MetaCache::with_slots(2);
        cache.mutate(&mut dev, 10, |d| d[0] = 0xEE).unwrap();
        assert_eq!(dev.write_count(), 0);
        cache.inspect(&mut dev, 12, |_| ()).unwrap();
        assert_eq!(dev.write_log(), &[10]);
    }

    #[test]
    fn clean_eviction_does_not_write() {
        let mut dev = CountingDevice::new(200);
        let mut cache = MetaCache::with_slots(2);
        cache.inspect(&mut dev, 10, |_| ()).unwrap();
        cache.inspect(&mut dev, 12, |_| ()).unwrap();
        assert_eq!(dev.write_count(), 0);
    }

    #[test]
    fn direct_mapping_shares_a_slot_between_conflicting_blocks() {
        let mut dev = CountingDevice::new(200);
        let mut cache = MetaCache::with_slots(4);
        cache.inspect(&mut dev, 5, |_| ()).unwrap();
        cache.inspect(&mut dev, 9, |_| ()).unwrap();
        assert_eq!(dev.read_count(), 2);
        cache.inspect(&mut dev, 5, |_| ()).unwrap();
        assert_eq!(dev.read_count(), 3);
    }

    #[test]
    fn invalidate_forces_reload_and_drops_dirty_data() {
        let mut dev = CountingDevice::new(1);
        let mut cache = MetaCache::new();
        cache.mutate(&mut dev, 0, |d| d[3] = 0x5C).unwrap();
        cache.invalidate(0);
        cache.inspect(&mut dev, 0, |d| assert_eq!(d[3], 0)).unwrap();
        assert_eq!(dev.write_count(), 0);
        assert_eq!(dev.read_count(), 2);
    }

    #[test]
    fn invalidate_of_other_resident_block_is_noop() {
        let mut dev = CountingDevice::new(200);
        let mut cache = MetaCache::with_slots(4);
        cache.mutate(&mut dev, 5, |d| d[0] = 9).unwrap();
        cache.invalidate(6);
        cache.inspect(&mut dev, 5, |d| assert_eq!(d[0], 9)).unwrap();
        assert_eq!(dev.read_count(), 1);
    }

    #[test]
    fn device_read_failure_propagates() {
        let mut dev = MemoryDevice::new(1);
        let mut cache = MetaCache::new();
        assert!(cache.inspect(&mut dev, 7, |_| ()).is_err());
    }
}
