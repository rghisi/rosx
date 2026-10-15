use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::collections::arena::GenerationalArena;
use crate::handle::{Handle, IpcConnectionHandle, IpcDataHandle};
use crate::ipc::errors::DataError;
use crate::task::TaskHandle;

const MAX_DATA_BYTES: usize = 64 * 1024;
const DATA_BUDGET_BYTES: usize = 256 * 1024;

pub(crate) struct DataObject {
    payload: Vec<u8>,
    connection: IpcConnectionHandle,
    writer: TaskHandle,
    read_cursor: usize,
}

pub(crate) struct IpcDataManager {
    objects: GenerationalArena<DataObject, 256>,
    outstanding: BTreeMap<TaskHandle, usize>,
    by_connection: BTreeMap<IpcConnectionHandle, Vec<Handle>>,
}

impl IpcDataManager {

    pub(crate) fn new() -> IpcDataManager {
        IpcDataManager {
            objects: GenerationalArena::new(),
            outstanding: BTreeMap::new(),
            by_connection: BTreeMap::new(),
        }
    }

    pub(crate) fn write(
        &mut self,
        caller: TaskHandle,
        connection: IpcConnectionHandle,
        payload: &[u8],
    ) -> Result<IpcDataHandle, DataError> {
        if payload.len() > MAX_DATA_BYTES {
            return Err(DataError::TooLarge);
        }
        let used = self.outstanding.get(&caller).copied().unwrap_or(0);
        if used + payload.len() > DATA_BUDGET_BYTES {
            return Err(DataError::BudgetExceeded);
        }
        let object = DataObject {
            payload: Vec::from(payload),
            connection,
            writer: caller,
            read_cursor: 0,
        };
        let handle = self.objects.add(object).map_err(|_| DataError::BudgetExceeded)?;
        self.outstanding.insert(caller, used + payload.len());
        self.by_connection.entry(connection).or_default().push(handle);
        Ok(IpcDataHandle::from(handle))
    }

    pub(crate) fn locate(
        &self,
        caller: TaskHandle,
        token: IpcDataHandle,
    ) -> Result<(IpcConnectionHandle, TaskHandle), DataError> {
        let object = self.objects.borrow(token.into()).map_err(|_| DataError::NotFound)?;
        if caller == object.writer {
            return Err(DataError::NotReadable);
        }
        Ok((object.connection, object.writer))
    }

    pub(crate) fn copy_out(&mut self, token: IpcDataHandle, dst: &mut [u8]) -> usize {
        let Ok(object) = self.objects.borrow_mut(token.into()) else {
            return 0;
        };
        let remaining = object.payload.len() - object.read_cursor;
        let count = core::cmp::min(dst.len(), remaining);
        dst[..count].copy_from_slice(&object.payload[object.read_cursor..object.read_cursor + count]);
        object.read_cursor += count;
        count
    }

    pub(crate) fn release(
        &mut self,
        caller: TaskHandle,
        token: IpcDataHandle,
    ) -> Result<(), DataError> {
        let object = self.objects.borrow(token.into()).map_err(|_| DataError::NotFound)?;
        if caller != object.writer {
            return Err(DataError::NotFound);
        }
        let writer = object.writer;
        let size = object.payload.len();
        self.refund(writer, size);
        self.evict(token.into());
        Ok(())
    }

    pub(crate) fn gc_connection(&mut self, connection: IpcConnectionHandle) {
        let Some(handles) = self.by_connection.remove(&connection) else {
            return;
        };
        for handle in handles {
            let reclaimable = self
                .objects
                .borrow(handle)
                .ok()
                .filter(|object| object.connection == connection)
                .map(|object| (object.writer, object.payload.len()));
            if let Some((writer, size)) = reclaimable {
                self.refund(writer, size);
                self.evict(handle);
            }
        }
    }

    fn refund(&mut self, writer: TaskHandle, bytes: usize) {
        let Some(used) = self.outstanding.get(&writer).copied() else {
            return;
        };
        let remaining = used.saturating_sub(bytes);
        if remaining == 0 {
            self.outstanding.remove(&writer);
        } else {
            self.outstanding.insert(writer, remaining);
        }
    }

    fn evict(&mut self, handle: Handle) -> Option<DataObject> {
        for handles in self.by_connection.values_mut() {
            handles.retain(|indexed| *indexed != handle);
        }
        self.by_connection.retain(|_, handles| !handles.is_empty());
        self.objects.remove(handle).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const WRITER_TASK: TaskHandle = TaskHandle::new(1, 1);
    const READER_TASK: TaskHandle = TaskHandle::new(2, 1);
    const INTRUDER_TASK: TaskHandle = TaskHandle::new(3, 1);
    const CONNECTION_A: IpcConnectionHandle = IpcConnectionHandle::new(1, 1);
    const CONNECTION_B: IpcConnectionHandle = IpcConnectionHandle::new(2, 1);

    fn outstanding(manager: &IpcDataManager, writer: TaskHandle) -> usize {
        manager.outstanding.get(&writer).copied().unwrap_or(0)
    }

    #[test]
    fn write_read_roundtrip_returns_full_payload() {
        let mut manager = IpcDataManager::new();
        let payload: Vec<u8> = (0..1000).map(|i| (i % 251) as u8).collect();
        let token = manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap();
        assert_eq!(manager.locate(READER_TASK, token), Ok((CONNECTION_A, WRITER_TASK)));
        let mut dst = vec![0u8; payload.len()];
        assert_eq!(manager.copy_out(token, &mut dst), payload.len());
        assert_eq!(dst, payload);
    }

    #[test]
    fn short_read_then_continuation_advances_cursor() {
        let mut manager = IpcDataManager::new();
        let payload: Vec<u8> = (0..300).map(|i| (i * 3 + 7) as u8).collect();
        let token = manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap();
        manager.locate(READER_TASK, token).unwrap();
        let mut first = vec![0u8; 128];
        assert_eq!(manager.copy_out(token, &mut first), 128);
        assert_eq!(first, payload[..128]);
        let mut second = vec![0u8; 128];
        assert_eq!(manager.copy_out(token, &mut second), 128);
        assert_eq!(second, payload[128..256]);
        let mut third = vec![0u8; 128];
        assert_eq!(manager.copy_out(token, &mut third), 44);
        assert_eq!(third[..44], payload[256..300]);
    }

    #[test]
    fn read_past_end_returns_zero() {
        let mut manager = IpcDataManager::new();
        let payload = [42u8; 10];
        let token = manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap();
        manager.locate(READER_TASK, token).unwrap();
        let mut dst = [0u8; 10];
        assert_eq!(manager.copy_out(token, &mut dst), 10);
        let mut extra = [0u8; 10];
        assert_eq!(manager.copy_out(token, &mut extra), 0);
        assert_eq!(extra, [0u8; 10]);
    }

    #[test]
    fn read_with_empty_dst_returns_zero() {
        let mut manager = IpcDataManager::new();
        let payload = [7u8; 10];
        let token = manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap();
        manager.locate(READER_TASK, token).unwrap();
        assert_eq!(manager.copy_out(token, &mut []), 0);
        let mut dst = [0u8; 10];
        assert_eq!(manager.copy_out(token, &mut dst), 10);
        assert_eq!(dst, payload);
    }

    #[test]
    fn write_over_max_data_bytes_is_too_large() {
        let mut manager = IpcDataManager::new();
        let at_limit = vec![0u8; MAX_DATA_BYTES];
        assert!(manager.write(WRITER_TASK, CONNECTION_A, &at_limit).is_ok());
        let over_limit = vec![0u8; MAX_DATA_BYTES + 1];
        assert_eq!(
            manager.write(WRITER_TASK, CONNECTION_A, &over_limit),
            Err(DataError::TooLarge)
        );
    }

    #[test]
    fn budget_exhaustion_then_refund_after_release() {
        let mut manager = IpcDataManager::new();
        let chunk = vec![9u8; MAX_DATA_BYTES];
        let mut tokens = Vec::new();
        for _ in 0..4 {
            tokens.push(manager.write(WRITER_TASK, CONNECTION_A, &chunk).unwrap());
        }
        assert_eq!(outstanding(&manager, WRITER_TASK), DATA_BUDGET_BYTES);
        assert_eq!(
            manager.write(WRITER_TASK, CONNECTION_A, &chunk),
            Err(DataError::BudgetExceeded)
        );
        assert_eq!(manager.release(WRITER_TASK, tokens[0]), Ok(()));
        assert!(manager.write(WRITER_TASK, CONNECTION_A, &chunk).is_ok());
    }

    #[test]
    fn budget_is_per_task() {
        let mut manager = IpcDataManager::new();
        let chunk = vec![5u8; MAX_DATA_BYTES];
        for _ in 0..4 {
            manager.write(WRITER_TASK, CONNECTION_A, &chunk).unwrap();
        }
        assert_eq!(outstanding(&manager, WRITER_TASK), DATA_BUDGET_BYTES);
        assert!(manager.write(READER_TASK, CONNECTION_A, &chunk).is_ok());
        assert_eq!(outstanding(&manager, READER_TASK), MAX_DATA_BYTES);
        assert_eq!(outstanding(&manager, WRITER_TASK), DATA_BUDGET_BYTES);
    }

    #[test]
    fn arena_capacity_exhaustion_maps_to_budget_exceeded() {
        let mut manager = IpcDataManager::new();
        for _ in 0..256 {
            manager.write(WRITER_TASK, CONNECTION_A, &[1u8]).unwrap();
        }
        assert_eq!(outstanding(&manager, WRITER_TASK), 256);
        assert_eq!(
            manager.write(WRITER_TASK, CONNECTION_A, &[1u8]),
            Err(DataError::BudgetExceeded)
        );
    }

    #[test]
    fn double_release_is_not_found() {
        let mut manager = IpcDataManager::new();
        let token = manager.write(WRITER_TASK, CONNECTION_A, b"twice").unwrap();
        assert_eq!(manager.release(WRITER_TASK, token), Ok(()));
        assert_eq!(manager.release(WRITER_TASK, token), Err(DataError::NotFound));
    }

    #[test]
    fn release_by_non_writer_is_not_found() {
        let mut manager = IpcDataManager::new();
        let token = manager.write(WRITER_TASK, CONNECTION_A, b"owned").unwrap();
        assert_eq!(manager.release(READER_TASK, token), Err(DataError::NotFound));
        manager.locate(READER_TASK, token).unwrap();
        let mut dst = [0u8; 5];
        assert_eq!(manager.copy_out(token, &mut dst), 5);
        assert_eq!(&dst, b"owned");
    }

    #[test]
    fn writer_cannot_read_own_object_is_not_readable() {
        let mut manager = IpcDataManager::new();
        let token = manager.write(WRITER_TASK, CONNECTION_A, b"mine").unwrap();
        assert_eq!(manager.locate(WRITER_TASK, token), Err(DataError::NotReadable));
    }

    #[test]
    fn stale_generation_token_rejected_after_slot_reuse() {
        let mut manager = IpcDataManager::new();
        let first = manager.write(WRITER_TASK, CONNECTION_A, &[1u8]).unwrap();
        manager.release(WRITER_TASK, first).unwrap();
        for _ in 0..255 {
            let filler = manager.write(WRITER_TASK, CONNECTION_A, &[2u8]).unwrap();
            manager.release(WRITER_TASK, filler).unwrap();
        }
        let second = manager.write(WRITER_TASK, CONNECTION_A, &[3u8]).unwrap();
        assert_eq!(second.index(), first.index());
        assert_eq!(second.generation(), first.generation() + 1);
        assert_eq!(manager.locate(INTRUDER_TASK, first), Err(DataError::NotFound));
        let mut dst = [0u8; 1];
        manager.locate(INTRUDER_TASK, second).unwrap();
        assert_eq!(manager.copy_out(second, &mut dst), 1);
        assert_eq!(dst, [3u8]);
    }

    #[test]
    fn index_entries_always_borrow_live_objects() {
        let mut manager = IpcDataManager::new();
        let payload = [8u8; 16];
        let a1 = manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap();
        let a2 = manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap();
        let a3 = manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap();
        let b1 = manager.write(READER_TASK, CONNECTION_B, &payload).unwrap();
        let b2 = manager.write(READER_TASK, CONNECTION_B, &payload).unwrap();
        assert_eq!(manager.locate(WRITER_TASK, b2), Ok((CONNECTION_B, READER_TASK)));
        manager.release(WRITER_TASK, a2).unwrap();
        manager.release(READER_TASK, b1).unwrap();
        manager.gc_connection(CONNECTION_B);
        assert_eq!(manager.locate(WRITER_TASK, b2), Err(DataError::NotFound));
        assert!(manager.by_connection.get(&CONNECTION_B).is_none());
        assert_eq!(
            manager.by_connection.get(&CONNECTION_A).unwrap(),
            &vec![Handle::from(a1), Handle::from(a3)]
        );
        for (connection, handles) in manager.by_connection.iter() {
            for handle in handles {
                let object = manager.objects.borrow(*handle).unwrap();
                assert_eq!(object.connection, *connection);
            }
        }
        assert_eq!(outstanding(&manager, READER_TASK), 0);
        assert_eq!(outstanding(&manager, WRITER_TASK), 32);
    }

    #[test]
    fn release_scrubs_index_and_refunds() {
        let mut manager = IpcDataManager::new();
        let payload = [3u8; 100];
        let first = manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap();
        let second = manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap();
        assert_eq!(outstanding(&manager, WRITER_TASK), 200);
        assert_eq!(manager.by_connection.get(&CONNECTION_A).unwrap().len(), 2);
        manager.release(WRITER_TASK, first).unwrap();
        assert_eq!(outstanding(&manager, WRITER_TASK), 100);
        let indexed = manager.by_connection.get(&CONNECTION_A).unwrap();
        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[0], Handle::from(second));
    }

    #[test]
    fn many_concurrent_objects_per_connection() {
        let mut manager = IpcDataManager::new();
        let mut tokens = Vec::new();
        let mut payloads = Vec::new();
        for i in 0..8u8 {
            let payload = vec![i; 32 + i as usize];
            tokens.push(manager.write(WRITER_TASK, CONNECTION_A, &payload).unwrap());
            payloads.push(payload);
        }
        for token in &tokens {
            assert_eq!(manager.locate(READER_TASK, *token), Ok((CONNECTION_A, WRITER_TASK)));
        }
        let mut cursors = [0usize; 8];
        let mut collected: Vec<Vec<u8>> = (0..8).map(|_| Vec::new()).collect();
        loop {
            let mut pending = false;
            for i in 0..8 {
                if cursors[i] < payloads[i].len() {
                    let want = core::cmp::min(7, payloads[i].len() - cursors[i]);
                    let mut chunk = [0u8; 7];
                    let got = manager.copy_out(tokens[i], &mut chunk[..want]);
                    assert_eq!(got, want);
                    collected[i].extend_from_slice(&chunk[..got]);
                    cursors[i] += got;
                    pending = true;
                }
            }
            if !pending {
                break;
            }
        }
        for i in 0..8 {
            assert_eq!(collected[i], payloads[i]);
            assert_eq!(manager.copy_out(tokens[i], &mut [0u8; 4]), 0);
        }
    }
}
