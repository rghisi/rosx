use core::cell::UnsafeCell;
use core::sync::atomic::{compiler_fence, Ordering};

pub(crate) struct KernelCell<T> {
    data: UnsafeCell<T>,
}

unsafe impl<T> Sync for KernelCell<T> {}

impl<T> KernelCell<T> {
    pub(crate) const fn new(data: T) -> Self {
        KernelCell {
            data: UnsafeCell::new(data),
        }
    }

    pub(crate) fn borrow(&self) -> &T {
        compiler_fence(Ordering::SeqCst);
        unsafe { &*self.data.get() }
    }

    #[allow(clippy::mut_from_ref)]
    pub(crate) fn borrow_mut(&self) -> &mut T {
        compiler_fence(Ordering::SeqCst);
        unsafe { &mut *self.data.get() }
    }
}
