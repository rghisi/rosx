use crate::ipc::message_buffer::{MessageBuffer, MessageRoute};
use collections::generational_arena::GenerationalArena;
use system::ipc::{IpcMessageError, IpcMessageHandle, MAX_MESSAGE_SIZE};

pub(crate) struct MessageBufferManager {
    buffers: GenerationalArena<MessageBuffer, 256>,
}

impl MessageBufferManager {
    pub(crate) fn new() -> Self {
        Self {
            buffers: GenerationalArena::new(),
        }
    }

    pub(crate) fn create(
        &mut self,
        data_size: usize,
        route: MessageRoute,
    ) -> Result<IpcMessageHandle, IpcMessageError> {
        if data_size > MAX_MESSAGE_SIZE {
            return Err(IpcMessageError::MessageTooLarge);
        }
        self.buffers
            .add(MessageBuffer::new_with_route(data_size, route))
            .map_err(|_| IpcMessageError::PoolExhausted)
    }

    pub(crate) fn write(
        &mut self,
        handle: IpcMessageHandle,
        bytes: &[u8],
    ) -> Result<(), IpcMessageError> {
        let buffer = self
            .buffers
            .borrow_mut(handle)
            .map_err(|_| IpcMessageError::BufferNotFound)?;
        buffer.write(bytes)
    }

    pub(crate) fn route_of(&self, handle: IpcMessageHandle) -> Option<MessageRoute> {
        self.buffers.borrow(handle).ok().map(|b| b.route())
    }

    pub(crate) fn seal(&mut self, handle: IpcMessageHandle) -> Result<(), IpcMessageError> {
        let buffer = self
            .buffers
            .borrow_mut(handle)
            .map_err(|_| IpcMessageError::BufferNotFound)?;
        buffer.seal()
    }

    pub(crate) fn read(
        &mut self,
        handle: IpcMessageHandle,
        dst: &mut [u8],
    ) -> Result<usize, IpcMessageError> {
        let buffer = self
            .buffers
            .borrow_mut(handle)
            .map_err(|_| IpcMessageError::BufferNotFound)?;
        buffer.read(dst)
    }

    pub(crate) fn dispose(&mut self, handle: IpcMessageHandle) -> Result<(), IpcMessageError> {
        self.buffers
            .remove(handle)
            .map(|_| ())
            .map_err(|_| IpcMessageError::BufferNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    use collections::generational_arena::Handle;

    fn dummy_route() -> MessageRoute {
        MessageRoute {
            connection: Handle::new(1, 1),
            destination: Handle::new(2, 1),
        }
    }

    #[test]
    fn create_write_seal_read_happy_path() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(16, dummy_route()).unwrap();
        assert!(manager.write(handle, b"hello kernel").is_ok());
        assert!(manager.seal(handle).is_ok());

        let mut dst = [0u8; 64];
        let copied = manager.read(handle, &mut dst).unwrap();
        assert_eq!(copied, 12);
        assert_eq!(&dst[..copied], b"hello kernel");
    }

    #[test]
    fn cursor_accumulates_across_writes_and_split_reads() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(8, dummy_route()).unwrap();
        assert!(manager.write(handle, b"AAAA").is_ok());
        assert!(manager.write(handle, b"BBBB").is_ok());
        assert!(manager.seal(handle).is_ok());

        let mut first = [0u8; 3];
        assert_eq!(manager.read(handle, &mut first).unwrap(), 3);
        assert_eq!(&first, b"AAA");

        let mut second = [0u8; 3];
        assert_eq!(manager.read(handle, &mut second).unwrap(), 3);
        assert_eq!(&second, b"ABB");

        let mut third = [0u8; 3];
        assert_eq!(manager.read(handle, &mut third).unwrap(), 2);
        assert_eq!(&third[..2], b"BB");

        let mut fourth = [0u8; 3];
        assert_eq!(manager.read(handle, &mut fourth).unwrap(), 0);
    }

    #[test]
    fn stale_generation_rejected_on_all_ops() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(4, dummy_route()).unwrap();
        assert!(manager.write(handle, b"data").is_ok());
        assert!(manager.dispose(handle).is_ok());

        assert_eq!(
            manager.write(handle, b"data"),
            Err(IpcMessageError::BufferNotFound)
        );
        assert_eq!(
            manager.read(handle, &mut [0u8; 4]),
            Err(IpcMessageError::BufferNotFound)
        );
        assert_eq!(manager.seal(handle), Err(IpcMessageError::BufferNotFound));
        assert_eq!(manager.dispose(handle), Err(IpcMessageError::BufferNotFound));
    }

    #[test]
    fn aba_stale_handle_fails_after_slot_recycle() {
        let mut manager = MessageBufferManager::new();
        let h1 = manager.create(4, dummy_route()).unwrap();

        let mut fillers: Vec<IpcMessageHandle> = Vec::new();
        for _ in 0..255 {
            fillers.push(manager.create(4, dummy_route()).unwrap());
        }
        assert!(manager.dispose(h1).is_ok());

        let h2 = manager.create(4, dummy_route()).unwrap();
        assert_eq!(h2.index, h1.index);
        assert_eq!(h2.generation, h1.generation + 1);

        assert_eq!(
            manager.write(h1, b"ghost"),
            Err(IpcMessageError::BufferNotFound)
        );
        assert_eq!(
            manager.read(h1, &mut [0u8; 5]),
            Err(IpcMessageError::BufferNotFound)
        );
        assert_eq!(manager.seal(h1), Err(IpcMessageError::BufferNotFound));
        assert_eq!(manager.dispose(h1), Err(IpcMessageError::BufferNotFound));
        assert!(manager.route_of(h1).is_none());

        assert!(manager.write(h2, b"fresh").is_ok());
        assert!(manager.seal(h2).is_ok());
        let mut dst = [0u8; 5];
        assert_eq!(manager.read(h2, &mut dst).unwrap(), 5);
        assert_eq!(&dst, b"fresh");
    }

    #[test]
    fn write_after_seal_returns_sealed() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(4, dummy_route()).unwrap();
        assert!(manager.seal(handle).is_ok());
        assert_eq!(manager.write(handle, b"late"), Err(IpcMessageError::Sealed));
    }

    #[test]
    fn read_before_seal_returns_unsealed() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(8, dummy_route()).unwrap();
        assert!(manager.write(handle, b"pending").is_ok());
        assert_eq!(
            manager.read(handle, &mut [0u8; 8]),
            Err(IpcMessageError::Unsealed)
        );
    }

    #[test]
    fn double_dispose_second_fails() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(4, dummy_route()).unwrap();
        assert!(manager.dispose(handle).is_ok());
        assert_eq!(manager.dispose(handle), Err(IpcMessageError::BufferNotFound));
    }

    #[test]
    fn seal_twice_second_returns_sealed() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(4, dummy_route()).unwrap();
        assert!(manager.seal(handle).is_ok());
        assert_eq!(manager.seal(handle), Err(IpcMessageError::Sealed));
    }

    #[test]
    fn short_read_then_eof() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(6, dummy_route()).unwrap();
        assert!(manager.write(handle, b"abcdef").is_ok());
        assert!(manager.seal(handle).is_ok());

        let mut small = [0u8; 4];
        assert_eq!(manager.read(handle, &mut small).unwrap(), 4);
        assert_eq!(&small, b"abcd");

        let mut rest = [0u8; 4];
        assert_eq!(manager.read(handle, &mut rest).unwrap(), 2);
        assert_eq!(&rest[..2], b"ef");

        let mut eof = [0u8; 4];
        assert_eq!(manager.read(handle, &mut eof).unwrap(), 0);
    }

    #[test]
    fn zero_size_create_and_zero_cap_read() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(0, dummy_route()).unwrap();
        assert!(manager.write(handle, b"").is_ok());

        let mut empty_dst = [0u8; 0];
        assert_eq!(
            manager.read(handle, &mut empty_dst),
            Err(IpcMessageError::Unsealed)
        );

        assert!(manager.seal(handle).is_ok());
        assert_eq!(manager.read(handle, &mut empty_dst).unwrap(), 0);
    }

    #[test]
    fn pool_exhaustion_then_recover_after_dispose() {
        let mut manager = MessageBufferManager::new();
        let mut handles: Vec<IpcMessageHandle> = Vec::new();
        for _ in 0..256 {
            handles.push(manager.create(4, dummy_route()).unwrap());
        }
        assert_eq!(manager.create(4, dummy_route()), Err(IpcMessageError::PoolExhausted));

        let victim = handles.remove(0);
        assert!(manager.dispose(victim).is_ok());
        assert!(manager.create(4, dummy_route()).is_ok());
        assert_eq!(manager.create(4, dummy_route()), Err(IpcMessageError::PoolExhausted));
    }

    #[test]
    fn create_rejects_data_size_above_cap() {
        let mut manager = MessageBufferManager::new();
        assert_eq!(
            manager.create(65, dummy_route()),
            Err(IpcMessageError::MessageTooLarge)
        );
        assert!(manager.create(1, dummy_route()).is_ok());
    }

    #[test]
    fn create_accepts_exactly_cap() {
        let mut manager = MessageBufferManager::new();
        assert!(manager.create(64, dummy_route()).is_ok());
    }

    #[test]
    fn write_beyond_cap_rejected_and_payload_untouched() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.create(0, dummy_route()).unwrap();
        assert!(manager.write(handle, &[0xAA; 40]).is_ok());
        assert_eq!(manager.write(handle, &[0xBB; 40]), Err(IpcMessageError::MessageTooLarge));
        assert!(manager.write(handle, &[0xCC; 24]).is_ok());
        assert_eq!(manager.write(handle, &[0xDD; 1]), Err(IpcMessageError::MessageTooLarge));
        assert!(manager.seal(handle).is_ok());

        let mut dst = [0u8; 64];
        assert_eq!(manager.read(handle, &mut dst).unwrap(), 64);

        let mut expected = [0u8; 64];
        expected[..40].copy_from_slice(&[0xAA; 40]);
        expected[40..].copy_from_slice(&[0xCC; 24]);
        assert_eq!(dst, expected);
    }

    #[test]
    fn route_of_returns_stamped_route() {
        let mut manager = MessageBufferManager::new();
        let route = MessageRoute {
            connection: Handle::new(3, 2),
            destination: Handle::new(5, 4),
        };
        let handle = manager.create(4, route).unwrap();
        let stamped = manager.route_of(handle).unwrap();
        assert_eq!(stamped.connection, Handle::new(3, 2));
        assert_eq!(stamped.destination, Handle::new(5, 4));
    }

    #[test]
    fn route_of_unknown_handle_returns_none() {
        let manager = MessageBufferManager::new();
        assert!(manager.route_of(Handle::new(99, 1)).is_none());
    }
}
