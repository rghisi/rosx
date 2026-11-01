pub mod bins;

#[cfg(all(feature = "slab-allocator", not(test)))]
mod shim {
    use super::bins::{Binner, SLAB_BYTES, SMALL_MAX, small_class_of};
    use crate::syscall::Syscall;
    use core::alloc::{GlobalAlloc, Layout};
    use core::cell::UnsafeCell;
    use core::ptr;
    use core::sync::atomic::{AtomicBool, Ordering};

    struct SharedBins {
        inner: UnsafeCell<Binner>,
    }

    unsafe impl Sync for SharedBins {}

    static ALLOC_LOCK: AtomicBool = AtomicBool::new(false);
    static SHARED_BINS: SharedBins = SharedBins {
        inner: UnsafeCell::new(Binner::new()),
    };

    fn acquire_lock() {
        while ALLOC_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            Syscall::task_yield();
        }
    }

    fn release_lock() {
        ALLOC_LOCK.store(false, Ordering::Release);
    }

    pub struct UsrAllocator;

    unsafe fn fill_bin_from_slab(binner: &mut Binner, class: usize) -> Option<usize> {
        let slab = Syscall::alloc(SLAB_BYTES, SMALL_MAX) as usize;
        if slab != 0 {
            unsafe { binner.adopt(class, slab) };
        }
        unsafe { binner.take(class) }
    }

    unsafe impl GlobalAlloc for UsrAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            match small_class_of(&layout) {
                Some(class) => {
                    acquire_lock();
                    let binner = unsafe { &mut *SHARED_BINS.inner.get() };
                    let chunk = match unsafe { binner.take(class) } {
                        Some(chunk) => Some(chunk),
                        None => unsafe { fill_bin_from_slab(binner, class) },
                    };
                    release_lock();
                    chunk.map_or(ptr::null_mut(), |chunk| chunk as *mut u8)
                }
                None => Syscall::alloc(layout.size(), layout.align()),
            }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            match small_class_of(&layout) {
                Some(class) => {
                    acquire_lock();
                    unsafe { (*SHARED_BINS.inner.get()).put(class, ptr as usize) };
                    release_lock();
                }
                None => Syscall::dealloc(ptr, layout.size(), layout.align()),
            }
        }
    }
}

#[cfg(all(feature = "slab-allocator", not(test)))]
pub use shim::UsrAllocator;
