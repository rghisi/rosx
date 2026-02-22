use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::time::Duration;
use system::future::FutureHandle;

pub(crate) struct Timer {
    next: BTreeMap<u64, Vec<FutureHandle>>
}

impl Timer {

    pub fn new() -> Self {
        Timer {
            next: BTreeMap::new(),
        }
    }

    pub fn add_sleep(&mut self, now: u64, sleep: Duration, future_handle: FutureHandle) {
        let deadline = now + (sleep.as_millis() as u64);
        self.next.entry(deadline).or_insert_with(Vec::new).push(future_handle)
    }

    pub fn pop_expired(&mut self, now: u64) -> Option<Vec<FutureHandle>> {
        let remaining = self.next.split_off(&(now + 1));
        let expired = core::mem::replace(&mut self.next, remaining);
        let handles: Vec<FutureHandle> = expired.into_values().flatten().collect();
        if handles.is_empty() { None } else { Some(handles) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collections::generational_arena::Handle;

    fn handle(index: u32) -> FutureHandle {
        Handle::new(index, 0)
    }

    #[test]
    fn pop_expired_returns_none_when_empty() {
        let mut timer = Timer::new();
        assert!(timer.pop_expired(100).is_none());
    }

    #[test]
    fn pop_expired_returns_none_when_no_entries_are_due() {
        let mut timer = Timer::new();
        timer.add_sleep(0, Duration::from_millis(50), handle(1));
        assert!(timer.pop_expired(49).is_none());
    }

    #[test]
    fn pop_expired_returns_handle_exactly_at_deadline() {
        let mut timer = Timer::new();
        timer.add_sleep(0, Duration::from_millis(50), handle(1));
        let expired = timer.pop_expired(50);
        assert_eq!(expired, Some(alloc::vec![handle(1)]));
    }

    #[test]
    fn pop_expired_returns_handle_past_deadline() {
        let mut timer = Timer::new();
        timer.add_sleep(0, Duration::from_millis(10), handle(1));
        let expired = timer.pop_expired(100);
        assert_eq!(expired, Some(alloc::vec![handle(1)]));
    }

    #[test]
    fn pop_expired_removes_returned_handles_from_timer() {
        let mut timer = Timer::new();
        timer.add_sleep(0, Duration::from_millis(10), handle(1));
        timer.pop_expired(100);
        assert!(timer.pop_expired(100).is_none());
    }

    #[test]
    fn pop_expired_preserves_future_handles() {
        let mut timer = Timer::new();
        timer.add_sleep(0, Duration::from_millis(10), handle(1));
        timer.add_sleep(0, Duration::from_millis(100), handle(2));
        timer.pop_expired(50);
        let still_pending = timer.pop_expired(200);
        assert_eq!(still_pending, Some(alloc::vec![handle(2)]));
    }

    #[test]
    fn pop_expired_flattens_multiple_handles_at_same_deadline() {
        let mut timer = Timer::new();
        timer.add_sleep(0, Duration::from_millis(10), handle(1));
        timer.add_sleep(0, Duration::from_millis(10), handle(2));
        let mut expired = timer.pop_expired(10).unwrap();
        expired.sort_by_key(|h| h.index);
        assert_eq!(expired, alloc::vec![handle(1), handle(2)]);
    }

    #[test]
    fn pop_expired_flattens_handles_across_multiple_deadlines() {
        let mut timer = Timer::new();
        timer.add_sleep(0, Duration::from_millis(10), handle(1));
        timer.add_sleep(0, Duration::from_millis(20), handle(2));
        let mut expired = timer.pop_expired(20).unwrap();
        expired.sort_by_key(|h| h.index);
        assert_eq!(expired, alloc::vec![handle(1), handle(2)]);
    }

    #[test]
    fn add_sleep_uses_now_as_base_for_deadline() {
        let mut timer = Timer::new();
        timer.add_sleep(1000, Duration::from_millis(50), handle(1));
        assert!(timer.pop_expired(1049).is_none());
        assert_eq!(timer.pop_expired(1050), Some(alloc::vec![handle(1)]));
    }
}