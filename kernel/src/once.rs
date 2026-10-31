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

    pub fn get_or_init(&self, initializer: impl FnOnce() -> T) -> &T {
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
                    (*self.value.get()).write(initializer());
                }
                self.state.store(READY, Ordering::Release);
            }
            Err(READY) => {}
            Err(_) => {
                while self.state.load(Ordering::Acquire) != READY {
                    core::hint::spin_loop();
                }
            }
        }
        // Safety: READY is stored only after the value was written, and the
        // Acquire ordering pairs with the Release store, so the value is a
        // valid T and no further mutation occurs.
        unsafe { (*self.value.get()).assume_init_ref() }
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

    #[test]
    fn get_or_init_initializes_and_returns_value() {
        let once = Once::new();
        assert_eq!(once.get_or_init(|| 42), &42);
        assert_eq!(once.get(), Some(&42));
    }

    #[test]
    fn get_or_init_runs_initializer_exactly_once() {
        use core::cell::Cell;
        let runs = Cell::new(0);
        let once = Once::new();
        assert_eq!(*once.get_or_init(|| {
            runs.set(runs.get() + 1);
            7
        }), 7);
        assert_eq!(*once.get_or_init(|| {
            runs.set(runs.get() + 1);
            8
        }), 7);
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn get_or_init_after_call_once_keeps_first_value() {
        let once = Once::new();
        once.call_once(|| 1);
        assert_eq!(once.get_or_init(|| 2), &1);
    }

    #[test]
    #[should_panic(expected = "called more than once")]
    fn call_once_after_get_or_init_panics() {
        let once = Once::new();
        once.get_or_init(|| 1);
        once.call_once(|| 2);
    }

    #[test]
    fn get_or_init_concurrent_returns_same_value_from_single_run() {
        use alloc::sync::Arc;
        use core::sync::atomic::{AtomicUsize, Ordering};
        let once = Arc::new(Once::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let handles: alloc::vec::Vec<std::thread::JoinHandle<u32>> = (0..4)
            .map(|_| {
                let once = Arc::clone(&once);
                let runs = Arc::clone(&runs);
                std::thread::spawn(move || {
                    *once.get_or_init(|| {
                        runs.fetch_add(1, Ordering::SeqCst);
                        5
                    })
                })
            })
            .collect();
        let values: alloc::vec::Vec<u32> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert!(values.iter().all(|value| *value == 5));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }
}