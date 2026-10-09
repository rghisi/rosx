use alloc::vec::Vec;
use collections::generational_arena::Handle;
use system::ipc::{IpcMessageError, MAX_MESSAGE_SIZE};

#[derive(Clone, Copy)]
pub(crate) enum BufferState {
    WriteOnly,
    ReadOnly,
}

#[derive(Clone, Copy)]
pub(crate) struct MessageRoute {
    pub(crate) connection: Handle,
    pub(crate) destination: Handle,
}

pub(crate) struct MessageBuffer {
    state: BufferState,
    payload: Vec<u8>,
    read_pos: usize,
    route: MessageRoute,
}

impl MessageBuffer {
    pub(crate) fn new_with_route(data_size: usize, route: MessageRoute) -> Self {
        Self {
            state: BufferState::WriteOnly,
            payload: Vec::with_capacity(data_size),
            read_pos: 0,
            route,
        }
    }

    pub(crate) fn route(&self) -> MessageRoute {
        self.route
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<(), IpcMessageError> {
        match self.state {
            BufferState::WriteOnly => {
                if self.payload.len() + bytes.len() > MAX_MESSAGE_SIZE {
                    return Err(IpcMessageError::MessageTooLarge);
                }
                self.payload.extend_from_slice(bytes);
                Ok(())
            }
            BufferState::ReadOnly => Err(IpcMessageError::Sealed),
        }
    }

    pub(crate) fn seal(&mut self) -> Result<(), IpcMessageError> {
        match self.state {
            BufferState::WriteOnly => {
                self.state = BufferState::ReadOnly;
                Ok(())
            }
            BufferState::ReadOnly => Err(IpcMessageError::Sealed),
        }
    }

    pub(crate) fn read(&mut self, dst: &mut [u8]) -> Result<usize, IpcMessageError> {
        match self.state {
            BufferState::WriteOnly => Err(IpcMessageError::Unsealed),
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
