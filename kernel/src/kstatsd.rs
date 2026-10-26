use crate::ipc::message_future::IpcMessageFuture;
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::memory::memory_manager::MemoryStats;
use system::ipc::{MESSAGE_PAYLOAD_BYTES, Message};
use system::kstats::{
    KSTATSD_SERVICE_NAME, KstatsRequest, MemorySnapshot, decode_request, encode_memory_snapshot,
};

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
        let stats = services().memory_manager.stats();
        let uptime_ms = kernel().get_system_time();
        let Some(payload) = reply_payload(request.data(), stats, uptime_ms) else {
            continue;
        };
        let caller = kernel().execution_state.current_task();
        let _ = services()
            .ipc_manager
            .borrow_mut()
            .send_message(caller, &Message::new(request.conn(), &payload));
    }
}

fn reply_payload(
    request_data: &[u8; MESSAGE_PAYLOAD_BYTES],
    stats: MemoryStats,
    uptime_ms: u64,
) -> Option<[u8; MESSAGE_PAYLOAD_BYTES]> {
    match decode_request(request_data)? {
        KstatsRequest::MemorySnapshot => Some(encode_memory_snapshot(&snapshot(stats, uptime_ms))),
    }
}

fn snapshot(stats: MemoryStats, uptime_ms: u64) -> MemorySnapshot {
    MemorySnapshot {
        used_bytes: stats.used_bytes as u64,
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
            used_bytes: 4096,
            total_bytes: 1024 * 1024,
            free_bytes: 512 * 1024,
            largest_free_block: 256 * 1024,
            free_block_count: 3,
        }
    }

    fn expected_snapshot() -> MemorySnapshot {
        MemorySnapshot {
            used_bytes: 4096,
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
