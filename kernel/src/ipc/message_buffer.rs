use alloc::vec::Vec;
use system::ipc::IpcBufferError;

#[derive(Clone, Copy)]
pub(crate) enum BufferState {
    WriteOnly,
    ReadOnly,
}

pub(crate) struct MessageBuffer {
    state: BufferState,
    payload: Vec<u8>,
    read_pos: usize,
}

impl MessageBuffer {
    pub(crate) fn new() -> Self {
        Self {
            state: BufferState::WriteOnly,
            payload: Vec::new(),
            read_pos: 0,
        }
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<(), IpcBufferError> {
        match self.state {
            BufferState::WriteOnly => {
                self.payload.extend_from_slice(bytes);
                Ok(())
            }
            BufferState::ReadOnly => Err(IpcBufferError::Sealed),
        }
    }

    pub(crate) fn seal(&mut self) -> Result<(), IpcBufferError> {
        match self.state {
            BufferState::WriteOnly => {
                self.state = BufferState::ReadOnly;
                Ok(())
            }
            BufferState::ReadOnly => Err(IpcBufferError::Sealed),
        }
    }

    pub(crate) fn read(&mut self, dst: &mut [u8]) -> Result<usize, IpcBufferError> {
        match self.state {
            BufferState::WriteOnly => Err(IpcBufferError::Unsealed),
            BufferState::ReadOnly => {
                let remaining = self.payload.len() - self.read_pos;
                let copied = core::cmp::min(dst.len(), remaining);
                dst[..copied].copy_from_slice(&self.payload[self.read_pos..self.read_pos + copied]);
                self.read_pos += copied;
                Ok(copied)
            }
        }
    }
}
