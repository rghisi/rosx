use collections::generational_arena::Handle;
use core::any::Any;

pub type FutureHandle = Handle;

pub trait Future: Send + Sync {
    fn is_completed(&self) -> bool;

    fn as_any(&self) -> &dyn Any;
}
