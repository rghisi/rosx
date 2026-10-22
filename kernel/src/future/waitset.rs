use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::any::Any;
use crate::handle::FutureHandle;
use super::Future;
use system::waitset::WaitEvent;

pub(crate) const WAITSET_CHILD_CAPACITY: usize = 16;
pub(crate) const WAITSET_PENDING_CAPACITY: usize = 16;

#[derive(Copy, Clone)]
pub(crate) struct ParentLink {
    pub(crate) parent: FutureHandle,
    pub(crate) tag: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AttachError {
    ParentNotFound,
    ChildInvalid,
    NestingForbidden,
    Full,
}

#[derive(Debug)]
pub(crate) enum WaitsetTake {
    Event(WaitEvent),
    Empty,
    Gone,
}

#[derive(Copy, Clone)]
pub(crate) struct WaitsetChild {
    child: FutureHandle,
    tag: u32,
}

pub(crate) struct WaitsetFuture {
    children: Vec<WaitsetChild>,
    pending: VecDeque<WaitEvent>,
}

impl WaitsetFuture {
    pub(crate) fn new() -> Self {
        Self {
            children: Vec::with_capacity(WAITSET_CHILD_CAPACITY),
            pending: VecDeque::with_capacity(WAITSET_PENDING_CAPACITY),
        }
    }

    pub(crate) fn add_child(&mut self, child: FutureHandle, tag: u32) {
        self.children.push(WaitsetChild { child, tag });
    }

    pub(crate) fn record(&mut self, event: WaitEvent) -> bool {
        let became_signalled = self.pending.is_empty();
        self.pending.push_back(event);
        became_signalled
    }

    pub(crate) fn take_pending(&mut self) -> Option<WaitEvent> {
        self.pending.pop_front()
    }

    pub(crate) fn remove_child(&mut self, child: FutureHandle) -> bool {
        let token = child.pack() as u32;
        let before = self.children.len();
        self.children.retain(|edge| edge.child != child);
        self.pending.retain(|event| event.child != token);
        self.children.len() != before
    }

    pub(crate) fn has_child(&self, child: FutureHandle) -> bool {
        self.children.iter().any(|edge| edge.child == child)
    }

    pub(crate) fn child_count(&self) -> usize {
        self.children.len()
    }

    pub(crate) fn child_handles(&self) -> Vec<FutureHandle> {
        self.children.iter().map(|edge| edge.child).collect()
    }
}

impl Future for WaitsetFuture {
    fn is_completed(&self) -> bool {
        !self.pending.is_empty()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn handle_at(index: u16, generation: u16) -> FutureHandle {
        FutureHandle::new(index, generation)
    }

    #[test]
    fn fresh_waitset_is_not_completed_and_has_no_pending_events() {
        let mut waitset = WaitsetFuture::new();
        assert!(!waitset.is_completed());
        assert_eq!(waitset.take_pending(), None);
    }

    #[test]
    fn first_record_reports_became_signalled_and_second_does_not() {
        let mut waitset = WaitsetFuture::new();
        assert!(waitset.record(WaitEvent { child: 7, tag: 11 }));
        assert!(!waitset.record(WaitEvent { child: 8, tag: 22 }));
        assert!(waitset.is_completed());
    }

    #[test]
    fn take_pending_drains_in_fifo_order() {
        let mut waitset = WaitsetFuture::new();
        waitset.record(WaitEvent { child: 1, tag: 10 });
        waitset.record(WaitEvent { child: 2, tag: 20 });
        assert_eq!(waitset.take_pending(), Some(WaitEvent { child: 1, tag: 10 }));
        assert_eq!(waitset.take_pending(), Some(WaitEvent { child: 2, tag: 20 }));
        assert_eq!(waitset.take_pending(), None);
        assert!(!waitset.is_completed());
    }

    #[test]
    fn add_child_grows_edges_and_has_child() {
        let mut waitset = WaitsetFuture::new();
        let child = handle_at(3, 0);
        assert!(!waitset.has_child(child));
        assert_eq!(waitset.child_count(), 0);
        waitset.add_child(child, 5);
        assert!(waitset.has_child(child));
        assert_eq!(waitset.child_count(), 1);
    }

    #[test]
    fn remove_child_drops_edge_and_purges_its_pending_events() {
        let mut waitset = WaitsetFuture::new();
        let child = handle_at(4, 2);
        let other = handle_at(5, 0);
        waitset.add_child(child, 1);
        waitset.add_child(other, 2);
        waitset.record(WaitEvent { child: child.pack() as u32, tag: 1 });
        waitset.record(WaitEvent { child: other.pack() as u32, tag: 2 });

        assert!(waitset.remove_child(child));
        assert!(!waitset.has_child(child));
        assert_eq!(waitset.take_pending(), Some(WaitEvent { child: other.pack() as u32, tag: 2 }));
        assert_eq!(waitset.take_pending(), None);
    }

    #[test]
    fn remove_child_of_unknown_edge_returns_false() {
        let mut waitset = WaitsetFuture::new();
        assert!(!waitset.remove_child(handle_at(40, 0)));
    }

    #[test]
    fn remove_child_ignores_recycled_generation_of_same_index() {
        let mut waitset = WaitsetFuture::new();
        let child = handle_at(6, 1);
        waitset.add_child(child, 3);
        assert!(!waitset.remove_child(handle_at(6, 2)));
        assert!(waitset.has_child(child));
    }

    #[test]
    fn child_handles_lists_attached_edges_in_order() {
        let mut waitset = WaitsetFuture::new();
        let first = handle_at(9, 0);
        let second = handle_at(10, 0);
        waitset.add_child(first, 1);
        waitset.add_child(second, 2);
        assert_eq!(waitset.child_handles(), vec![first, second]);
    }
}
