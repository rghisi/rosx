use alloc::boxed::Box;
use core::any::Any;
use collections::generational_arena::Handle;

pub type FutureHandle = Handle;

pub trait Future: Send + Sync {
    fn is_completed(&self) -> bool;

    /// Mark this future as completed. Default no-op; override for
    /// notification-driven futures (e.g. TimeFuture) where the
    /// TimerManager flips the flag.
    fn complete(&mut self) {}

    fn as_any(&self) -> &dyn Any;

    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync>;
}