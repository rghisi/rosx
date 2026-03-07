use core::cell::UnsafeCell;
use core::sync::atomic::{compiler_fence, Ordering};

pub struct KernelCell<T> {
    data: UnsafeCell<T>,
}

unsafe impl<T> Sync for KernelCell<T> {}

impl<T> KernelCell<T> {
    pub const fn new(data: T) -> Self {
        KernelCell {
            data: UnsafeCell::new(data),
        }
    }

    pub fn borrow(&self) -> &T {
        compiler_fence(Ordering::SeqCst);
        unsafe { &*self.data.get() }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn borrow_mut(&self) -> &mut T {
        compiler_fence(Ordering::SeqCst);
        unsafe { &mut *self.data.get() }
    }
}
