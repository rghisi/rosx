use crate::handle::{IpcConnectionHandle};
use crate::ipc::message_future::IpcMessageFuture;
use crate::irq_guard::IrqCell;
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::memory::memory_manager::MemoryStats;
use crate::task::{TaskHandle, TaskState};
use alloc::vec;
use alloc::vec::Vec;
use system::ipc::{MESSAGE_PAYLOAD_BYTES, Message, encode_data_token};
use system::kstats::{
    KSTATSD_SERVICE_NAME, KstatsRequest, MemorySnapshot, ProcessRecord, decode_request,
    encode_memory_snapshot, encode_process_table, process_table_bytes_for,
    PROCESS_STATE_BLOCKED, PROCESS_STATE_CREATED, PROCESS_STATE_READY, PROCESS_STATE_RUNNING,
};

static PROCESS_TABLE_SECTION: IrqCell = IrqCell::new("kstatsd_process_table");

fn wire_state(state: TaskState) -> u8 {
    match state {
        TaskState::Running => PROCESS_STATE_RUNNING,
        TaskState::Ready => PROCESS_STATE_READY,
        TaskState::Blocked => PROCESS_STATE_BLOCKED,
        _ => PROCESS_STATE_CREATED,
    }
}

pub fn main() {
    let Ok(binding) = services()
        .ipc_manager
        .borrow_mut()
        .bind_service(KSTATSD_SERVICE_NAME)
    else {
        return;
    };
    loop {
        let accepted = services()
            .ipc_manager
            .borrow_mut()
            .accept_message_async(binding);
        let Some(received) = kernel().wait::<IpcMessageFuture>(accepted) else {
            continue;
        };
        let Ok(request) = received.result() else {
            continue;
        };
        let self_task = kernel().execution_state.current_task();
        if let Some(KstatsRequest::ProcessTable) = decode_request(request.data()) {
            reply_process_table(request.conn(), self_task);
            continue;
        }
        let stats = services().memory_manager.stats();
        let uptime_ms = kernel().get_system_time();
        let Some(payload) = reply_payload(request.data(), stats, uptime_ms) else {
            continue;
        };
        let _ = services()
            .ipc_manager
            .borrow_mut()
            .send_message(self_task, &Message::new(request.conn(), &payload));
    }
}

fn reply_process_table(conn: u32, writer: TaskHandle) {
    let Some(connection) = IpcConnectionHandle::from_packed(conn as usize) else {
        return;
    };
    let (rows, mem_bytes) = {
        let _section = PROCESS_TABLE_SECTION.acquire();
        let rows = services().task_manager.borrow().snapshot_processes();
        let mem_bytes: Vec<usize> = rows
            .iter()
            .map(|row| services().memory_manager.group_bytes(row.group))
            .collect();
        (rows, mem_bytes)
    };
    let records: Vec<ProcessRecord> = rows
        .iter()
        .zip(mem_bytes.iter())
        .map(|(row, bytes)| ProcessRecord {
            pid: row.group as u32,
            state: wire_state(row.state),
            thread_count: row.thread_count,
            mem_bytes: *bytes as u64,
            name: &row.name[..row.name_len],
        })
        .collect();
    let mut blob = vec![0u8; process_table_bytes_for(records.len())];
    let Some(encoded_len) = encode_process_table(&records, &mut blob) else {
        return;
    };
    blob.truncate(encoded_len);
    let Ok(token) = services()
        .ipc_manager
        .borrow_mut()
        .write_data(writer, connection, &blob)
    else {
        return;
    };
    let mut payload = [0u8; MESSAGE_PAYLOAD_BYTES];
    encode_data_token(&mut payload, token.pack() as u32);
    let _ = services()
        .ipc_manager
        .borrow_mut()
        .send_message(writer, &Message::new(conn, &payload));
}

fn reply_payload(
    request_data: &[u8; MESSAGE_PAYLOAD_BYTES],
    stats: MemoryStats,
    uptime_ms: u64,
) -> Option<[u8; MESSAGE_PAYLOAD_BYTES]> {
    match decode_request(request_data)? {
        KstatsRequest::MemorySnapshot => Some(encode_memory_snapshot(&snapshot(stats, uptime_ms))),
        KstatsRequest::ProcessTable => None,
    }
}

fn snapshot(stats: MemoryStats, uptime_ms: u64) -> MemorySnapshot {
    MemorySnapshot {
        total_bytes: stats.total_bytes as u64,
        free_bytes: stats.free_bytes as u64,
        largest_free_block: stats.largest_free_block as u64,
        free_block_count: stats.free_block_count as u64,
        uptime_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::IpcConnectionHandle;
    use crate::kernel_services::init as init_services;
    use crate::task::TaskHandle;
    use system::kstats::{SNAPSHOT_SCHEMA_VERSION, decode_memory_snapshot, encode_memory_request};

    fn stats_fixture() -> MemoryStats {
        MemoryStats {
            total_bytes: 1024 * 1024,
            free_bytes: 512 * 1024,
            largest_free_block: 256 * 1024,
            free_block_count: 3,
        }
    }

    fn expected_snapshot() -> MemorySnapshot {
        MemorySnapshot {
            total_bytes: 1024 * 1024,
            free_bytes: 512 * 1024,
            largest_free_block: 256 * 1024,
            free_block_count: 3,
            uptime_ms: 1234,
        }
    }

    #[test]
    fn snapshot_request_is_answered_with_the_encoded_snapshot() {
        let reply = reply_payload(&encode_memory_request(), stats_fixture(), 1234).unwrap();
        assert_eq!(decode_memory_snapshot(&reply), Some(expected_snapshot()));
    }

    #[test]
    fn unknown_op_gets_no_reply() {
        let mut request = encode_memory_request();
        request[0] = 99;
        assert!(reply_payload(&request, stats_fixture(), 1234).is_none());
    }

    #[test]
    fn snapshot_request_roundtrips_through_the_ipc_manager() {
        init_services();
        let client = TaskHandle::new(1, 1);
        let server = TaskHandle::new(2, 1);

        let binding = services()
            .ipc_manager
            .borrow_mut()
            .bind_service(KSTATSD_SERVICE_NAME)
            .unwrap();
        let connection = services()
            .ipc_manager
            .borrow_mut()
            .connect(KSTATSD_SERVICE_NAME, client)
            .unwrap();
        services()
            .ipc_manager
            .borrow_mut()
            .send_message(client, &Message::new(connection.pack() as u32, &encode_memory_request()))
            .unwrap();

        let accepted = services()
            .ipc_manager
            .borrow_mut()
            .accept_message_async(binding);
        let future = services()
            .future_registry
            .borrow_mut()
            .consume(accepted)
            .unwrap();
        let received = future.into_any().downcast::<IpcMessageFuture>().ok().unwrap();
        let request = received.result().unwrap();
        assert_eq!(request.conn(), connection.pack() as u32);

        let payload = reply_payload(request.data(), stats_fixture(), 1234).unwrap();
        services()
            .ipc_manager
            .borrow_mut()
            .send_message(server, &Message::new(request.conn(), &payload))
            .unwrap();

        let pending_reply = services()
            .ipc_manager
            .borrow_mut()
            .receive_message_async(connection);
        let future = services()
            .future_registry
            .borrow_mut()
            .consume(pending_reply)
            .unwrap();
        let received = future.into_any().downcast::<IpcMessageFuture>().ok().unwrap();
        let reply = received.result().unwrap();
        assert_eq!(reply.data()[0], SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(decode_memory_snapshot(reply.data()), Some(expected_snapshot()));
    }

    #[test]
    fn reply_to_disconnected_client_is_dropped_without_panic() {
        init_services();
        let client = TaskHandle::new(3, 1);
        let server = TaskHandle::new(4, 1);
        services()
            .ipc_manager
            .borrow_mut()
            .bind_service("KSTATSD-DISCONNECT-TEST")
            .unwrap();
        let connection = services()
            .ipc_manager
            .borrow_mut()
            .connect("KSTATSD-DISCONNECT-TEST", client)
            .unwrap();
        services().ipc_manager.borrow_mut().disconnect(connection);
        let stale = IpcConnectionHandle::from_packed(connection.pack()).unwrap();
        let result = services()
            .ipc_manager
            .borrow_mut()
            .send_message(server, &Message::new(stale.pack() as u32, &encode_memory_request()));
        assert!(result.is_err());
    }
}
