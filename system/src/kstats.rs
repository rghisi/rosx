use crate::ipc::MESSAGE_PAYLOAD_BYTES;

pub const KSTATSD_SERVICE_NAME: &str = "KSTATSD";
pub const SNAPSHOT_SCHEMA_VERSION: u8 = 2;
const OP_MEMORY_SNAPSHOT: u8 = 0;
const OP_PROCESS_TABLE: u8 = 1;

pub const PROCESS_TABLE_SCHEMA_VERSION: u8 = 1;

pub const PROCESS_STATE_CREATED: u8 = 0;
pub const PROCESS_STATE_BLOCKED: u8 = 1;
pub const PROCESS_STATE_READY: u8 = 2;
pub const PROCESS_STATE_RUNNING: u8 = 3;

pub const PROCESS_NAME_BYTES: usize = 32;
pub const PROCESS_ENTRY_BYTES: usize = 48;
pub const PROCESS_TABLE_HEADER_BYTES: usize = 5;
pub const MAX_PROCESS_ENTRIES: usize = 256;
pub const MAX_PROCESS_TABLE_BYTES: usize =
    PROCESS_TABLE_HEADER_BYTES + MAX_PROCESS_ENTRIES * PROCESS_ENTRY_BYTES;

const ENTRY_PID_OFFSET: usize = 0;
const ENTRY_STATE_OFFSET: usize = 4;
const ENTRY_THREAD_COUNT_OFFSET: usize = 5;
const ENTRY_MEM_BYTES_OFFSET: usize = 8;
const ENTRY_NAME_OFFSET: usize = 16;

const OP_OFFSET: usize = 0;
const VERSION_OFFSET: usize = 0;
const TOTAL_BYTES_OFFSET: usize = 1;
const FREE_BYTES_OFFSET: usize = 9;
const LARGEST_FREE_BLOCK_OFFSET: usize = 17;
const FREE_BLOCK_COUNT_OFFSET: usize = 25;
const UPTIME_MS_OFFSET: usize = 33;
pub const SNAPSHOT_BYTES: usize = 41;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KstatsRequest {
    MemorySnapshot,
    ProcessTable,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ProcessRecord<'a> {
    pub pid: u32,
    pub state: u8,
    pub thread_count: u16,
    pub mem_bytes: u64,
    pub name: &'a [u8],
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub largest_free_block: u64,
    pub free_block_count: u64,
    pub uptime_ms: u64,
}

pub fn encode_memory_request() -> [u8; MESSAGE_PAYLOAD_BYTES] {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[OP_OFFSET] = OP_MEMORY_SNAPSHOT;
    data
}

pub fn decode_request(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Option<KstatsRequest> {
    match data[OP_OFFSET] {
        OP_MEMORY_SNAPSHOT => Some(KstatsRequest::MemorySnapshot),
        OP_PROCESS_TABLE => Some(KstatsRequest::ProcessTable),
        _ => None,
    }
}

pub fn encode_memory_snapshot(snapshot: &MemorySnapshot) -> [u8; MESSAGE_PAYLOAD_BYTES] {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[VERSION_OFFSET] = SNAPSHOT_SCHEMA_VERSION;
    write_u64(&mut data, TOTAL_BYTES_OFFSET, snapshot.total_bytes);
    write_u64(&mut data, FREE_BYTES_OFFSET, snapshot.free_bytes);
    write_u64(&mut data, LARGEST_FREE_BLOCK_OFFSET, snapshot.largest_free_block);
    write_u64(&mut data, FREE_BLOCK_COUNT_OFFSET, snapshot.free_block_count);
    write_u64(&mut data, UPTIME_MS_OFFSET, snapshot.uptime_ms);
    data
}

pub fn decode_memory_snapshot(data: &[u8; MESSAGE_PAYLOAD_BYTES]) -> Option<MemorySnapshot> {
    if data[VERSION_OFFSET] != SNAPSHOT_SCHEMA_VERSION {
        return None;
    }
    Some(MemorySnapshot {
        total_bytes: read_u64(data, TOTAL_BYTES_OFFSET),
        free_bytes: read_u64(data, FREE_BYTES_OFFSET),
        largest_free_block: read_u64(data, LARGEST_FREE_BLOCK_OFFSET),
        free_block_count: read_u64(data, FREE_BLOCK_COUNT_OFFSET),
        uptime_ms: read_u64(data, UPTIME_MS_OFFSET),
    })
}

pub fn encode_process_table_request() -> [u8; MESSAGE_PAYLOAD_BYTES] {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[OP_OFFSET] = OP_PROCESS_TABLE;
    data
}

pub fn process_table_bytes_for(entry_count: usize) -> usize {
    PROCESS_TABLE_HEADER_BYTES + entry_count * PROCESS_ENTRY_BYTES
}

pub fn encode_process_table(entries: &[ProcessRecord<'_>], out: &mut [u8]) -> Option<usize> {
    let count = entries.len();
    if count > MAX_PROCESS_ENTRIES {
        return None;
    }
    let needed = process_table_bytes_for(count);
    if out.len() < needed {
        return None;
    }
    out[..PROCESS_TABLE_HEADER_BYTES].fill(0);
    out[VERSION_OFFSET] = PROCESS_TABLE_SCHEMA_VERSION;
    out[1..5].copy_from_slice(&(count as u32).to_le_bytes());
    for (index, entry) in entries.iter().enumerate() {
        if entry.name.len() > PROCESS_NAME_BYTES {
            return None;
        }
        let base = process_table_bytes_for(index);
        let row = &mut out[base..base + PROCESS_ENTRY_BYTES];
        row.fill(0);
        row[ENTRY_PID_OFFSET..ENTRY_PID_OFFSET + 4].copy_from_slice(&entry.pid.to_le_bytes());
        row[ENTRY_STATE_OFFSET] = entry.state;
        row[ENTRY_THREAD_COUNT_OFFSET..ENTRY_THREAD_COUNT_OFFSET + 2]
            .copy_from_slice(&entry.thread_count.to_le_bytes());
        row[ENTRY_MEM_BYTES_OFFSET..ENTRY_MEM_BYTES_OFFSET + 8]
            .copy_from_slice(&entry.mem_bytes.to_le_bytes());
        row[ENTRY_NAME_OFFSET..ENTRY_NAME_OFFSET + entry.name.len()].copy_from_slice(entry.name);
    }
    Some(needed)
}

pub fn decode_process_table_count(blob: &[u8]) -> Option<usize> {
    if blob.len() < PROCESS_TABLE_HEADER_BYTES || blob[VERSION_OFFSET] != PROCESS_TABLE_SCHEMA_VERSION {
        return None;
    }
    let count = u32::from_le_bytes(blob[1..5].try_into().unwrap()) as usize;
    if count > MAX_PROCESS_ENTRIES || blob.len() < process_table_bytes_for(count) {
        return None;
    }
    Some(count)
}

pub fn decode_process_entry(blob: &[u8], index: usize) -> Option<ProcessRecord<'_>> {
    let count = decode_process_table_count(blob)?;
    if index >= count {
        return None;
    }
    let base = process_table_bytes_for(index);
    let row = blob.get(base..base + PROCESS_ENTRY_BYTES)?;
    let name_field = &row[ENTRY_NAME_OFFSET..];
    let name_len = name_field
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);
    Some(ProcessRecord {
        pid: u32::from_le_bytes(
            row[ENTRY_PID_OFFSET..ENTRY_PID_OFFSET + 4].try_into().unwrap(),
        ),
        state: row[ENTRY_STATE_OFFSET],
        thread_count: u16::from_le_bytes(
            row[ENTRY_THREAD_COUNT_OFFSET..ENTRY_THREAD_COUNT_OFFSET + 2].try_into().unwrap(),
        ),
        mem_bytes: u64::from_le_bytes(
            row[ENTRY_MEM_BYTES_OFFSET..ENTRY_MEM_BYTES_OFFSET + 8].try_into().unwrap(),
        ),
        name: &name_field[..name_len],
    })
}

fn write_u64(data: &mut [u8; MESSAGE_PAYLOAD_BYTES], offset: usize, value: u64) {
    data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u64(data: &[u8; MESSAGE_PAYLOAD_BYTES], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn sample_snapshot() -> MemorySnapshot {
        MemorySnapshot {
            total_bytes: 0x0202_0202_0202_0202,
            free_bytes: 0x0303_0303_0303_0303,
            largest_free_block: 0x0404_0404_0404_0404,
            free_block_count: 0x0505_0505_0505_0505,
            uptime_ms: 0x0606_0606_0606_0606,
        }
    }

    #[test]
    fn snapshot_encoding_fits_message_payload() {
        assert!(SNAPSHOT_BYTES <= MESSAGE_PAYLOAD_BYTES);
    }

    #[test]
    fn memory_request_roundtrips() {
        let request = encode_memory_request();
        assert_eq!(decode_request(&request), Some(KstatsRequest::MemorySnapshot));
    }

    #[test]
    fn request_with_unknown_op_is_rejected() {
        let mut request = encode_memory_request();
        request[OP_OFFSET] = 99;
        assert_eq!(decode_request(&request), None);
    }

    #[test]
    fn snapshot_roundtrips_through_payload() {
        let snapshot = sample_snapshot();
        let data = encode_memory_snapshot(&snapshot);
        assert_eq!(decode_memory_snapshot(&data), Some(snapshot));
    }

    #[test]
    fn snapshot_fields_are_written_little_endian_at_their_offsets() {
        let snapshot = sample_snapshot();
        let data = encode_memory_snapshot(&snapshot);
        assert_eq!(data[VERSION_OFFSET], SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(&data[TOTAL_BYTES_OFFSET..FREE_BYTES_OFFSET], &snapshot.total_bytes.to_le_bytes());
        assert_eq!(&data[UPTIME_MS_OFFSET..SNAPSHOT_BYTES], &snapshot.uptime_ms.to_le_bytes());
    }

    #[test]
    fn reserved_trailing_bytes_stay_zero() {
        let data = encode_memory_snapshot(&sample_snapshot());
        assert!(data[SNAPSHOT_BYTES..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn decode_rejects_unsupported_schema_version() {
        let mut data = encode_memory_snapshot(&sample_snapshot());
        data[VERSION_OFFSET] = SNAPSHOT_SCHEMA_VERSION + 1;
        assert_eq!(decode_memory_snapshot(&data), None);
    }

    #[test]
    fn zeroed_snapshot_roundtrips() {
        let snapshot = MemorySnapshot {
            total_bytes: 0,
            free_bytes: 0,
            largest_free_block: 0,
            free_block_count: 0,
            uptime_ms: 0,
        };
        let data = encode_memory_snapshot(&snapshot);
        assert_eq!(decode_memory_snapshot(&data), Some(snapshot));
    }

    #[test]
    fn process_table_request_roundtrips() {
        let request = encode_process_table_request();
        assert_eq!(decode_request(&request), Some(KstatsRequest::ProcessTable));
    }

    #[test]
    fn memory_and_process_table_requests_are_distinguishable() {
        assert_ne!(encode_memory_request()[OP_OFFSET], encode_process_table_request()[OP_OFFSET]);
    }

    fn sample_records<'a>() -> Vec<ProcessRecord<'a>> {
        vec![
            ProcessRecord {
                pid: 1,
                state: PROCESS_STATE_RUNNING,
                thread_count: 1,
                mem_bytes: 4096,
                name: b"init",
            },
            ProcessRecord {
                pid: 7,
                state: PROCESS_STATE_BLOCKED,
                thread_count: 4,
                mem_bytes: 131_072,
                name: b"vfsd",
            },
        ]
    }

    #[test]
    fn process_table_roundtrips_through_the_blob() {
        let records = sample_records();
        let mut blob = vec![0u8; process_table_bytes_for(records.len())];
        let encoded = encode_process_table(&records, &mut blob).unwrap();
        assert_eq!(encoded, blob.len());
        assert_eq!(decode_process_table_count(&blob), Some(records.len()));
        assert_eq!(decode_process_entry(&blob, 0), Some(records[0]));
        assert_eq!(decode_process_entry(&blob, 1), Some(records[1]));
    }

    #[test]
    fn empty_process_table_roundtrips() {
        let mut blob = vec![0u8; process_table_bytes_for(0)];
        let encoded = encode_process_table(&[], &mut blob).unwrap();
        assert_eq!(encoded, PROCESS_TABLE_HEADER_BYTES);
        assert_eq!(decode_process_table_count(&blob), Some(0));
        assert_eq!(decode_process_entry(&blob, 0), None);
    }

    #[test]
    fn process_table_rejects_truncated_blob() {
        let records = sample_records();
        let mut blob = vec![0u8; process_table_bytes_for(records.len())];
        encode_process_table(&records, &mut blob).unwrap();
        blob.truncate(blob.len() - 1);
        assert_eq!(decode_process_table_count(&blob), None);
    }

    #[test]
    fn process_table_rejects_unsupported_schema_version() {
        let records = sample_records();
        let mut blob = vec![0u8; process_table_bytes_for(records.len())];
        encode_process_table(&records, &mut blob).unwrap();
        blob[0] = PROCESS_TABLE_SCHEMA_VERSION + 1;
        assert_eq!(decode_process_table_count(&blob), None);
    }

    #[test]
    fn process_table_rejects_out_of_range_entry_index() {
        let records = sample_records();
        let mut blob = vec![0u8; process_table_bytes_for(records.len())];
        encode_process_table(&records, &mut blob).unwrap();
        assert_eq!(decode_process_entry(&blob, records.len()), None);
    }

    #[test]
    fn process_table_out_buffer_must_fit_the_header() {
        assert_eq!(encode_process_table(&[], &mut [0u8; 4]), None);
    }

    #[test]
    fn process_table_rejects_overlong_name() {
        let record = ProcessRecord {
            pid: 1,
            state: PROCESS_STATE_READY,
            thread_count: 1,
            mem_bytes: 0,
            name: &[b'x'; PROCESS_NAME_BYTES + 1],
        };
        let mut blob = vec![0u8; process_table_bytes_for(1)];
        assert_eq!(encode_process_table(&[record], &mut blob), None);
    }

    #[test]
    fn process_table_name_field_is_zero_padded_and_trimmed() {
        let record = ProcessRecord {
            pid: 42,
            state: PROCESS_STATE_CREATED,
            thread_count: 0,
            mem_bytes: 0,
            name: b"pi",
        };
        let mut blob = vec![0u8; process_table_bytes_for(1)];
        encode_process_table(&[record], &mut blob).unwrap();
        let entry = decode_process_entry(&blob, 0).unwrap();
        assert_eq!(entry.name, b"pi");
        assert_eq!(entry.pid, 42);
    }

    #[test]
    fn process_table_rejects_count_beyond_capacity() {
        let mut blob = vec![0u8; process_table_bytes_for(1)];
        blob[0] = PROCESS_TABLE_SCHEMA_VERSION;
        blob[1..5].copy_from_slice(&((MAX_PROCESS_ENTRIES + 1) as u32).to_le_bytes());
        assert_eq!(decode_process_table_count(&blob), None);
    }
}
