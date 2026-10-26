use crate::ipc::MESSAGE_PAYLOAD_BYTES;

pub const KSTATSD_SERVICE_NAME: &str = "KSTATSD";
pub const SNAPSHOT_SCHEMA_VERSION: u8 = 1;
const OP_MEMORY_SNAPSHOT: u8 = 0;

const OP_OFFSET: usize = 0;
const VERSION_OFFSET: usize = 0;
const USED_BYTES_OFFSET: usize = 1;
const TOTAL_BYTES_OFFSET: usize = 9;
const FREE_BYTES_OFFSET: usize = 17;
const LARGEST_FREE_BLOCK_OFFSET: usize = 25;
const FREE_BLOCK_COUNT_OFFSET: usize = 33;
const UPTIME_MS_OFFSET: usize = 41;
pub const SNAPSHOT_BYTES: usize = 49;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KstatsRequest {
    MemorySnapshot,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub used_bytes: u64,
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
        _ => None,
    }
}

pub fn encode_memory_snapshot(snapshot: &MemorySnapshot) -> [u8; MESSAGE_PAYLOAD_BYTES] {
    let mut data = [0u8; MESSAGE_PAYLOAD_BYTES];
    data[VERSION_OFFSET] = SNAPSHOT_SCHEMA_VERSION;
    write_u64(&mut data, USED_BYTES_OFFSET, snapshot.used_bytes);
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
        used_bytes: read_u64(data, USED_BYTES_OFFSET),
        total_bytes: read_u64(data, TOTAL_BYTES_OFFSET),
        free_bytes: read_u64(data, FREE_BYTES_OFFSET),
        largest_free_block: read_u64(data, LARGEST_FREE_BLOCK_OFFSET),
        free_block_count: read_u64(data, FREE_BLOCK_COUNT_OFFSET),
        uptime_ms: read_u64(data, UPTIME_MS_OFFSET),
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

    fn sample_snapshot() -> MemorySnapshot {
        MemorySnapshot {
            used_bytes: 0x0101_0101_0101_0101,
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
        assert_eq!(&data[USED_BYTES_OFFSET..TOTAL_BYTES_OFFSET], &snapshot.used_bytes.to_le_bytes());
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
            used_bytes: 0,
            total_bytes: 0,
            free_bytes: 0,
            largest_free_block: 0,
            free_block_count: 0,
            uptime_ms: 0,
        };
        let data = encode_memory_snapshot(&snapshot);
        assert_eq!(decode_memory_snapshot(&data), Some(snapshot));
    }
}
