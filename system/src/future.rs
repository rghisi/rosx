use alloc::boxed::Box;
use core::any::Any;
use collections::generational_arena::Handle;
use crate::ipc::{IpcMessage, IpcReceiveError};

pub type FutureHandle = Handle;

pub enum FutureResult {
    IpcMessage(Result<IpcMessage, IpcReceiveError>),
    Void,
}

pub trait Future: Send + Sync {
    fn is_completed(&self) -> bool;

    fn into_result(self: Box<Self>) -> FutureResult;

    fn as_any(&self) -> &dyn Any;

    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync>;
}