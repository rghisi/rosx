use crate::ipc::message_buffer::MessageBuffer;
use collections::generational_arena::GenerationalArena;
use system::ipc::{IpcBufferError, IpcBufferHandle};

pub(crate) struct MessageBufferManager {
    buffers: GenerationalArena<MessageBuffer, 256>,
}

impl MessageBufferManager {
    pub(crate) fn new() -> Self {
        Self {
            buffers: GenerationalArena::new(),
        }
    }

    pub(crate) fn alloc(&mut self) -> Result<IpcBufferHandle, IpcBufferError> {
        self.buffers
            .add(MessageBuffer::new())
            .map_err(|_| IpcBufferError::PoolExhausted)
    }

    pub(crate) fn write(
        &mut self,
        handle: IpcBufferHandle,
        bytes: &[u8],
    ) -> Result<(), IpcBufferError> {
        let buffer = self
            .buffers
            .borrow_mut(handle)
            .map_err(|_| IpcBufferError::BufferNotFound)?;
        buffer.write(bytes)
    }

    pub(crate) fn seal(&mut self, handle: IpcBufferHandle) -> Result<(), IpcBufferError> {
        let buffer = self
            .buffers
            .borrow_mut(handle)
            .map_err(|_| IpcBufferError::BufferNotFound)?;
        buffer.seal()
    }

    pub(crate) fn read(
        &mut self,
        handle: IpcBufferHandle,
        dst: &mut [u8],
    ) -> Result<usize, IpcBufferError> {
        let buffer = self
            .buffers
            .borrow_mut(handle)
            .map_err(|_| IpcBufferError::BufferNotFound)?;
        buffer.read(dst)
    }

    pub(crate) fn dispose(&mut self, handle: IpcBufferHandle) -> Result<(), IpcBufferError> {
        self.buffers
            .remove(handle)
            .map(|_| ())
            .map_err(|_| IpcBufferError::BufferNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn alloc_write_seal_read_happy_path() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.alloc().unwrap();
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
        let handle = manager.alloc().unwrap();
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
        let handle = manager.alloc().unwrap();
        assert!(manager.write(handle, b"data").is_ok());
        assert!(manager.dispose(handle).is_ok());

        assert_eq!(
            manager.write(handle, b"data"),
            Err(IpcBufferError::BufferNotFound)
        );
        assert_eq!(
            manager.read(handle, &mut [0u8; 4]),
            Err(IpcBufferError::BufferNotFound)
        );
        assert_eq!(manager.seal(handle), Err(IpcBufferError::BufferNotFound));
        assert_eq!(manager.dispose(handle), Err(IpcBufferError::BufferNotFound));
    }

    #[test]
    fn aba_stale_handle_fails_after_slot_recycle() {
        let mut manager = MessageBufferManager::new();
        let h1 = manager.alloc().unwrap();

        let mut fillers: Vec<IpcBufferHandle> = Vec::new();
        for _ in 0..255 {
            fillers.push(manager.alloc().unwrap());
        }
        assert!(manager.dispose(h1).is_ok());

        let h2 = manager.alloc().unwrap();
        assert_eq!(h2.index, h1.index);
        assert_eq!(h2.generation, h1.generation + 1);

        assert_eq!(
            manager.write(h1, b"ghost"),
            Err(IpcBufferError::BufferNotFound)
        );
        assert_eq!(
            manager.read(h1, &mut [0u8; 5]),
            Err(IpcBufferError::BufferNotFound)
        );
        assert_eq!(manager.seal(h1), Err(IpcBufferError::BufferNotFound));
        assert_eq!(manager.dispose(h1), Err(IpcBufferError::BufferNotFound));

        assert!(manager.write(h2, b"fresh").is_ok());
        assert!(manager.seal(h2).is_ok());
        let mut dst = [0u8; 5];
        assert_eq!(manager.read(h2, &mut dst).unwrap(), 5);
        assert_eq!(&dst, b"fresh");
    }

    #[test]
    fn write_after_seal_returns_sealed() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.alloc().unwrap();
        assert!(manager.seal(handle).is_ok());
        assert_eq!(manager.write(handle, b"late"), Err(IpcBufferError::Sealed));
    }

    #[test]
    fn read_before_seal_returns_unsealed() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.alloc().unwrap();
        assert!(manager.write(handle, b"pending").is_ok());
        assert_eq!(
            manager.read(handle, &mut [0u8; 8]),
            Err(IpcBufferError::Unsealed)
        );
    }

    #[test]
    fn double_dispose_second_fails() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.alloc().unwrap();
        assert!(manager.dispose(handle).is_ok());
        assert_eq!(manager.dispose(handle), Err(IpcBufferError::BufferNotFound));
    }

    #[test]
    fn seal_twice_second_returns_sealed() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.alloc().unwrap();
        assert!(manager.seal(handle).is_ok());
        assert_eq!(manager.seal(handle), Err(IpcBufferError::Sealed));
    }

    #[test]
    fn short_read_then_eof() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.alloc().unwrap();
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
    fn zero_size_write_and_zero_cap_read() {
        let mut manager = MessageBufferManager::new();
        let handle = manager.alloc().unwrap();
        assert!(manager.write(handle, b"").is_ok());

        let mut empty_dst = [0u8; 0];
        assert_eq!(
            manager.read(handle, &mut empty_dst),
            Err(IpcBufferError::Unsealed)
        );

        assert!(manager.seal(handle).is_ok());
        assert_eq!(manager.read(handle, &mut empty_dst).unwrap(), 0);
    }

    #[test]
    fn pool_exhaustion_then_recover_after_dispose() {
        let mut manager = MessageBufferManager::new();
        let mut handles: Vec<IpcBufferHandle> = Vec::new();
        for _ in 0..256 {
            handles.push(manager.alloc().unwrap());
        }
        assert_eq!(manager.alloc(), Err(IpcBufferError::PoolExhausted));

        let victim = handles.remove(0);
        assert!(manager.dispose(victim).is_ok());
        assert!(manager.alloc().is_ok());
        assert_eq!(manager.alloc(), Err(IpcBufferError::PoolExhausted));
    }
}
