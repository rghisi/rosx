use core::any::Any;
use collections::generational_arena::Handle;

pub type FutureHandle = Handle<u32, u32>;

pub trait Future: Send + Sync {
    fn is_completed(&self) -> bool;

    fn as_any(&self) -> &dyn Any;
}