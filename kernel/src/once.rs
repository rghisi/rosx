use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU8, Ordering};

const UNINIT: u8 = 0;
const INITIALIZING: u8 = 1;
const READY: u8 = 2;

pub struct Once<T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
}

// Safety: Once<T> guarantees that the inner value is only written once (during
// call_once) and all subsequent access is read-only. The AtomicU8 state field
// provides the synchronization barrier. T must be Send (safe to transfer
// between threads) and Sync (safe to share references).
unsafe impl<T: Send + Sync> Sync for Once<T> {}
unsafe impl<T: Send> Send for Once<T> {}

impl<T> Once<T> {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(UNINIT),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub fn call_once(&self, f: impl FnOnce() -> T) {
        match self.state.compare_exchange(
            UNINIT,
            INITIALIZING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                // Safety: we are the only thread that transitioned from UNINIT
                // to INITIALIZING, so we have exclusive access to the value.
                unsafe {
                    (*self.value.get()).write(f());
                }
                self.state.store(READY, Ordering::Release);
            }
            Err(_) => panic!("Once::call_once called more than once"),
        }
    }

    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == READY {
            // Safety: state is READY, meaning call_once completed and wrote a
            // valid T. No further mutation occurs, so a shared reference is safe.
            Some(unsafe { (*self.value.get()).assume_init_ref() })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn new_returns_none() {
        let once: Once<i32> = Once::new();
        assert!(once.get().is_none());
    }

    #[test]
    fn call_once_then_get() {
        let once = Once::new();
        once.call_once(|| 42);
        assert_eq!(once.get(), Some(&42));
    }

    #[test]
    #[should_panic(expected = "called more than once")]
    fn double_call_once_panics() {
        let once = Once::new();
        once.call_once(|| 1);
        once.call_once(|| 2);
    }

    #[test]
    fn works_with_non_copy_types() {
        let once = Once::new();
        once.call_once(|| vec![1, 2, 3]);
        let v: &Vec<i32> = once.get().unwrap();
        assert_eq!(v, &vec![1, 2, 3]);
    }

    #[test]
    fn get_returns_stable_reference() {
        let once = Once::new();
        once.call_once(|| 99);
        let r1 = once.get().unwrap();
        let r2 = once.get().unwrap();
        assert!(core::ptr::eq(r1, r2));
    }
}