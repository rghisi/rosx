use core::any::Any;
use collections::generational_arena::Handle;
use crate::future::Future;

pub type IpcServerHandle = Handle;

#[derive(Debug)]
pub enum IpcError {
    ServerCannotBeAdded,
    ServerNotFound,
}

pub struct IpcSendMessage {
    pub value: u32
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct IpcReply {
    pub value: u32,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct IpcReplyFuture {
    pub reply: Option<IpcReply>,
}

impl IpcReplyFuture {
    pub fn complete(&mut self, reply: IpcReply) {
        self.reply.replace(reply);
    }
}

impl Future for IpcReplyFuture {
    fn is_completed(&self) -> bool {
        self.reply.is_some()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}