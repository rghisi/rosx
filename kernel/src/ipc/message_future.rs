use alloc::boxed::Box;
use core::any::Any;
use crate::future::Future;
use crate::ipc::errors::IpcReceiveError;
use crate::task::TaskHandle;
use system::ipc::Message;

pub(crate) struct IpcMessageFuture {
    message: Option<Message>,
    error: Option<IpcReceiveError>,
    owner: TaskHandle,
}

impl IpcMessageFuture {
    pub(crate) fn owned_by(owner: TaskHandle) -> Self {
        Self { message: None, error: None, owner }
    }

    pub(crate) fn with_message(owner: TaskHandle, message: Message) -> Self {
        Self { message: Some(message), error: None, owner }
    }

    pub(crate) fn with_error(owner: TaskHandle, error: IpcReceiveError) -> Self {
        Self { message: None, error: Some(error), owner }
    }

    pub(crate) fn complete(&mut self, message: Message) {
        self.message = Some(message);
    }

    pub(crate) fn result(&self) -> Result<Message, IpcReceiveError> {
        if let Some(err) = self.error {
            return Err(err);
        }
        self.message.ok_or(IpcReceiveError::NoMessagesAvailable)
    }
}

impl Future for IpcMessageFuture {
    fn is_completed(&self) -> bool {
        self.message.is_some() || self.error.is_some()
    }

    fn owner(&self) -> TaskHandle {
        self.owner
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync> {
        self
    }
}
