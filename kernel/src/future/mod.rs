pub(crate) mod waitset;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::any::Any;
use crate::collections::arena::{Error, GenerationalArena};
use crate::handle::FutureHandle;
use crate::task::TaskHandle;
use crate::kernel_services::services;
use crate::ports::driven::ForWakingTasks;
use self::waitset::{AttachError, ParentLink, WaitsetFuture, WaitsetTake, WAITSET_CHILD_CAPACITY};
use system::waitset::WaitEvent;

pub(crate) const FUTURE_SLOTS: usize = 1024;

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

pub struct TimeFuture {
    completed: bool,
}

impl TimeFuture {
    pub fn new() -> TimeFuture {
        TimeFuture { completed: false }
    }
}
impl Future for TimeFuture {
    fn is_completed(&self) -> bool {
        self.completed
    }

    fn complete(&mut self) {
        self.completed = true;
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

pub struct TaskCompletionFuture {
    task_handle: TaskHandle,
}

impl TaskCompletionFuture {
    pub fn new(task_handle: TaskHandle) -> Self {
        Self { task_handle }
    }
}

impl Future for TaskCompletionFuture {
    fn is_completed(&self) -> bool {
        services().task_manager.borrow().get_state(self.task_handle) == crate::task::TaskState::Terminated
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

pub(crate) struct TaskFuture {
    pub(crate) task_handle: TaskHandle,
    pub(crate) future_handle: FutureHandle,
}

impl TaskFuture {
    pub(crate) fn is_completed(&self) -> bool {
        services().future_registry.borrow_mut().get(self.future_handle).unwrap_or(true)
    }
}

pub struct FutureRegistry {
    arena: GenerationalArena<Box<dyn Future + Send + Sync>, FUTURE_SLOTS>,
    waiters: BTreeMap<FutureHandle, Vec<TaskHandle>>,
    parents_of: Vec<Vec<ParentLink>>,
    wake_controller: &'static dyn ForWakingTasks,
}

struct NoopWakeController;
impl ForWakingTasks for NoopWakeController {
    fn wake_tasks(&self, _handles: Vec<TaskHandle>) {}
}

static DEFAULT_WAKE_CONTROLLER: NoopWakeController = NoopWakeController;

fn empty_parent_rows() -> Vec<Vec<ParentLink>> {
    let mut rows = Vec::with_capacity(FUTURE_SLOTS);
    for _ in 0..FUTURE_SLOTS {
        rows.push(Vec::new());
    }
    rows
}

impl Default for FutureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FutureRegistry {
    pub fn new() -> Self {
        Self {
            arena: GenerationalArena::new(),
            waiters: BTreeMap::new(),
            parents_of: empty_parent_rows(),
            wake_controller: &DEFAULT_WAKE_CONTROLLER,
        }
    }

    pub fn new_with_wake_controller(wake_controller: &'static dyn ForWakingTasks) -> Self {
        Self {
            arena: GenerationalArena::new(),
            waiters: BTreeMap::new(),
            parents_of: empty_parent_rows(),
            wake_controller,
        }
    }

    pub fn register(&mut self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle> {
        self.arena.add(future).ok().map(FutureHandle::from)
    }

    pub fn register_waiter(&mut self, future_handle: FutureHandle, task_handle: TaskHandle) {
        self.waiters.entry(future_handle).or_default().push(task_handle);
    }

    pub fn get(&mut self, handle: FutureHandle) -> Option<bool> {
        if let Ok(future) = self.arena.borrow_mut(handle.into()) {
            Some(future.is_completed())
        } else {
            None
        }
    }

    pub fn consume(&mut self, handle: FutureHandle) -> Result<Box<dyn Future + Send + Sync>, Error> {
        let removed = self.arena.remove(handle.into())?;
        if let Some(row_slot) = self.parents_of.get_mut(handle.index() as usize) {
            let row = core::mem::take(row_slot);
            for link in row {
                if let Ok(future) = self.arena.borrow_mut(link.parent.into()) {
                    if let Some(waitset) = future.as_any_mut().downcast_mut::<WaitsetFuture>() {
                        waitset.remove_child(handle);
                    }
                }
            }
        }
        Ok(removed)
    }

    pub fn borrow_mut(&mut self, handle: FutureHandle) -> Result<&mut Box<dyn Future + Send + Sync>, Error> {
        self.arena.borrow_mut(handle.into())
    }

    pub fn replace(&mut self, handle: FutureHandle, future: Box<dyn Future + Send + Sync>) -> Result<FutureHandle, Error> {
        self.arena.replace(handle.into(), future).map(FutureHandle::from)
    }

    pub fn notify(&mut self, handle: FutureHandle) {
        let waiters = self.waiters.remove(&handle).unwrap_or_default();
        self.wake_controller.wake_tasks(waiters);
        self.cascade_to_waitsets(handle);
    }

    pub(crate) fn is_waitset(&mut self, handle: FutureHandle) -> bool {
        match self.arena.borrow_mut(handle.into()) {
            Ok(future) => future.as_any().downcast_ref::<WaitsetFuture>().is_some(),
            Err(_) => false,
        }
    }

    pub(crate) fn attach(&mut self, set: FutureHandle, child: FutureHandle, tag: u32) -> Result<(), AttachError> {
        let parent_is_waitset = match self.arena.borrow_mut(set.into()) {
            Ok(future) => future.as_any().downcast_ref::<WaitsetFuture>().is_some(),
            Err(_) => return Err(AttachError::ParentNotFound),
        };
        if !parent_is_waitset {
            return Err(AttachError::ParentNotFound);
        }
        let child_completed = match self.arena.borrow(child.into()) {
            Ok(future) => {
                if future.as_any().downcast_ref::<WaitsetFuture>().is_some() {
                    return Err(AttachError::NestingForbidden);
                }
                future.is_completed()
            }
            Err(_) => return Err(AttachError::ChildInvalid),
        };
        {
            let parent = match self.arena.borrow_mut(set.into()) {
                Ok(future) => match future.as_any_mut().downcast_mut::<WaitsetFuture>() {
                    Some(waitset) => waitset,
                    None => return Err(AttachError::ParentNotFound),
                },
                Err(_) => return Err(AttachError::ParentNotFound),
            };
            if parent.has_child(child) {
                return Err(AttachError::ChildInvalid);
            }
            if parent.child_count() == WAITSET_CHILD_CAPACITY {
                return Err(AttachError::Full);
            }
            parent.add_child(child);
            if child_completed {
                parent.record(WaitEvent { child: child.pack() as u32, tag });
            }
        }
        self.parents_of[child.index() as usize].push(ParentLink { parent: set, tag });
        if child_completed {
            self.notify(set);
        }
        Ok(())
    }

    pub(crate) fn detach(&mut self, set: FutureHandle, child: FutureHandle) -> Result<(), AttachError> {
        let edge_removed = match self.arena.borrow_mut(set.into()) {
            Ok(future) => match future.as_any_mut().downcast_mut::<WaitsetFuture>() {
                Some(waitset) => waitset.remove_child(child),
                None => return Err(AttachError::ParentNotFound),
            },
            Err(_) => return Err(AttachError::ParentNotFound),
        };
        if !edge_removed {
            return Err(AttachError::ChildInvalid);
        }
        if let Some(row) = self.parents_of.get_mut(child.index() as usize) {
            row.retain(|link| link.parent != set);
        }
        Ok(())
    }

    pub(crate) fn take_pending(&mut self, set: FutureHandle) -> WaitsetTake {
        loop {
            let event = match self.arena.borrow_mut(set.into()) {
                Ok(future) => match future.as_any_mut().downcast_mut::<WaitsetFuture>() {
                    Some(waitset) => match waitset.take_pending() {
                        Some(event) => event,
                        None => return WaitsetTake::Empty,
                    },
                    None => return WaitsetTake::Gone,
                },
                Err(_) => return WaitsetTake::Gone,
            };
            let live = match FutureHandle::from_packed(event.child as usize) {
                Some(child) => self.is_live(child),
                None => false,
            };
            if live {
                return WaitsetTake::Event(event);
            }
        }
    }

    pub(crate) fn drop_waitset(&mut self, set: FutureHandle) {
        let children = match self.arena.borrow_mut(set.into()) {
            Ok(future) => match future.as_any_mut().downcast_mut::<WaitsetFuture>() {
                Some(waitset) => waitset.child_handles(),
                None => return,
            },
            Err(_) => return,
        };
        for child in children {
            let _ = self.detach(set, child);
        }
        self.notify(set);
        let _ = self.consume(set);
    }

    fn is_live(&mut self, handle: FutureHandle) -> bool {
        self.arena.borrow(handle.into()).is_ok()
    }

    fn cascade_to_waitsets(&mut self, handle: FutureHandle) {
        let index = handle.index() as usize;
        let mut row = match self.parents_of.get_mut(index) {
            Some(slot) => core::mem::take(slot),
            None => return,
        };
        if row.is_empty() {
            return;
        }
        if !self.is_live(handle) {
            self.parents_of[index] = row;
            return;
        }
        let child_token = handle.pack() as u32;
        let mut satisfied_end = 0usize;
        let mut cursor = 0usize;
        while cursor < row.len() {
            let link = row[cursor];
            let recorded = match self.arena.borrow_mut(link.parent.into()) {
                Ok(future) => match future.as_any_mut().downcast_mut::<WaitsetFuture>() {
                    Some(waitset) => Some(waitset.record(WaitEvent { child: child_token, tag: link.tag })),
                    None => None,
                },
                Err(_) => None,
            };
            match recorded {
                Some(became_signalled) => {
                    if became_signalled {
                        row.swap(cursor, satisfied_end);
                        satisfied_end += 1;
                    }
                    cursor += 1;
                }
                None => {
                    row.swap_remove(cursor);
                }
            }
        }
        self.parents_of[index] = row;
        let mut wake_cursor = 0usize;
        while wake_cursor < satisfied_end {
            let parent = match self.parents_of[index].get(wake_cursor) {
                Some(link) => link.parent,
                None => break,
            };
            self.notify(parent);
            wake_cursor += 1;
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_services::{init as init_services, services};
    use crate::task::Task;
    use crate::task::TaskState;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| {
            init_services();
            crate::scheduler::wire_scheduler_for_tests();
        });
    }

    struct DummyFuture;
    impl Future for DummyFuture {
        fn is_completed(&self) -> bool {
            false
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

    #[test]
    fn notify_moves_waiters_to_ready() {
        setup();

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);

        let future_handle = services().future_registry.borrow_mut().register(Box::new(DummyFuture)).unwrap();
        services().future_registry.borrow_mut().register_waiter(future_handle, handle);

        services().future_registry.borrow_mut().notify(future_handle);

        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
    }

    #[test]
    fn notify_does_not_double_wake() {
        setup();

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);

        let future_handle = services().future_registry.borrow_mut().register(Box::new(DummyFuture)).unwrap();
        services().future_registry.borrow_mut().register_waiter(future_handle, handle);

        // First notify moves task to Ready
        services().future_registry.borrow_mut().notify(future_handle);
        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);

        // Second notify does nothing (no waiters left)
        services().future_registry.borrow_mut().notify(future_handle);
        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
    }

    // === TimeFuture tests ===

    #[test]
    fn time_future_starts_not_completed() {
        let future = TimeFuture::new();
        assert!(!future.is_completed());
    }

    #[test]
    fn time_future_complete_flips_flag() {
        let mut future = TimeFuture::new();
        future.complete();
        assert!(future.is_completed());
    }

    #[test]
    fn time_future_multiple_completes_are_idempotent() {
        let mut future = TimeFuture::new();
        future.complete();
        future.complete();
        future.complete();
        assert!(future.is_completed());
    }

    #[test]
    fn future_registry_can_complete_time_future_via_trait() {
        setup();

        let mut registry = FutureRegistry::new();
        let time_future = Box::new(TimeFuture::new());
        let handle = registry.register(time_future).unwrap();

        assert!(!registry.get(handle).unwrap());

        // Complete through the trait object (simulates timer firing)
        if let Ok(future) = registry.borrow_mut(handle) {
            future.complete();
        }

        assert!(registry.get(handle).unwrap());
    }

    #[test]
    fn complete_default_noop_does_not_affect_other_futures() {
        setup();

        let mut registry = FutureRegistry::new();
        let dummy = Box::new(DummyFuture);
        let handle = registry.register(dummy).unwrap();

        assert!(!registry.get(handle).unwrap());

        // Calling complete() on a non-TimeFuture is a no-op
        if let Ok(future) = registry.borrow_mut(handle) {
            future.complete();
        }

        assert!(!registry.get(handle).unwrap());
    }

    // === Waitset registry tests ===

    struct DummyCompletedFuture;
    impl Future for DummyCompletedFuture {
        fn is_completed(&self) -> bool {
            true
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

    fn register_waitset(registry: &mut FutureRegistry) -> FutureHandle {
        registry.register(Box::new(WaitsetFuture::new())).unwrap()
    }

    fn expect_event(take: WaitsetTake, expected_child: FutureHandle, expected_tag: u32) {
        match take {
            WaitsetTake::Event(event) => {
                assert_eq!(event.child, expected_child.pack() as u32);
                assert_eq!(event.tag, expected_tag);
            }
            WaitsetTake::Empty => panic!("expected recorded event, queue was empty"),
            WaitsetTake::Gone => panic!("expected recorded event, waitset was gone"),
        }
    }

    fn waitset_of(registry: &mut FutureRegistry, handle: FutureHandle) -> &WaitsetFuture {
        registry
            .borrow_mut(handle)
            .unwrap()
            .as_any()
            .downcast_ref::<WaitsetFuture>()
            .unwrap()
    }

    #[test]
    fn fresh_waitset_reports_not_completed_and_empty_take() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        assert_eq!(registry.get(set), Some(false));
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
    }

    #[test]
    fn attach_rejects_absent_parent_and_leaves_child_row_untouched() {
        let mut registry = FutureRegistry::new();
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        let absent = FutureHandle::new(700, 9);
        assert_eq!(registry.attach(absent, child, 0), Err(AttachError::ParentNotFound));
        assert!(registry.parents_of[child.index() as usize].is_empty());
    }

    #[test]
    fn attach_rejects_parent_that_is_not_a_waitset() {
        let mut registry = FutureRegistry::new();
        let plain = registry.register(Box::new(DummyFuture)).unwrap();
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        assert_eq!(registry.attach(plain, child, 0), Err(AttachError::ParentNotFound));
        assert!(registry.parents_of[child.index() as usize].is_empty());
    }

    #[test]
    fn attach_rejects_absent_child() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let absent = FutureHandle::new(701, 9);
        assert_eq!(registry.attach(set, absent, 0), Err(AttachError::ChildInvalid));
        assert!(registry.parents_of[absent.index() as usize].is_empty());
        assert_eq!(waitset_of(&mut registry, set).child_count(), 0);
    }

    #[test]
    fn attach_rejects_waitset_child_and_self_attach() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let other_set = register_waitset(&mut registry);
        assert_eq!(registry.attach(set, other_set, 0), Err(AttachError::NestingForbidden));
        assert_eq!(registry.attach(set, set, 7), Err(AttachError::NestingForbidden));
        assert!(registry.parents_of[other_set.index() as usize].is_empty());
        assert!(registry.parents_of[set.index() as usize].is_empty());
    }

    #[test]
    fn attach_rejects_duplicate_child() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        assert_eq!(registry.attach(set, child, 1), Ok(()));
        assert_eq!(registry.attach(set, child, 2), Err(AttachError::ChildInvalid));
        assert_eq!(registry.parents_of[child.index() as usize].len(), 1);
        assert_eq!(waitset_of(&mut registry, set).child_count(), 1);
    }

    #[test]
    fn attach_rejects_child_beyond_capacity() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        for _ in 0..WAITSET_CHILD_CAPACITY {
            let child = registry.register(Box::new(DummyFuture)).unwrap();
            assert_eq!(registry.attach(set, child, 0), Ok(()));
        }
        let overflow = registry.register(Box::new(DummyFuture)).unwrap();
        assert_eq!(registry.attach(set, overflow, 0), Err(AttachError::Full));
        assert!(registry.parents_of[overflow.index() as usize].is_empty());
    }

    #[test]
    fn attach_latches_already_completed_child() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let done = registry.register(Box::new(DummyCompletedFuture)).unwrap();
        assert_eq!(registry.attach(set, done, 42), Ok(()));
        assert_eq!(registry.get(set), Some(true));
        expect_event(registry.take_pending(set), done, 42);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
    }

    #[test]
    fn is_waitset_distinguishes_waitsets_plain_futures_and_absent_handles() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let plain = registry.register(Box::new(DummyFuture)).unwrap();
        assert!(registry.is_waitset(set));
        assert!(!registry.is_waitset(plain));
        assert!(!registry.is_waitset(FutureHandle::new(703, 9)));
    }

    #[test]
    fn notify_child_records_event_and_wakes_parent_waiter() {
        setup();

        let mut registry = services().future_registry.borrow_mut();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        registry.attach(set, child, 9).unwrap();

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);
        registry.register_waiter(set, handle);

        registry.notify(child);

        expect_event(registry.take_pending(set), child, 9);
        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
    }

    #[test]
    fn fan_in_records_into_every_attached_set() {
        let mut registry = FutureRegistry::new();
        let first = register_waitset(&mut registry);
        let second = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        registry.attach(first, child, 1).unwrap();
        registry.attach(second, child, 2).unwrap();

        registry.notify(child);

        expect_event(registry.take_pending(first), child, 1);
        expect_event(registry.take_pending(second), child, 2);
        assert!(matches!(registry.take_pending(first), WaitsetTake::Empty));
    }

    #[test]
    fn notify_after_parent_consumed_prunes_link_without_recording() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        registry.attach(set, child, 3).unwrap();
        registry.consume(set).unwrap();

        registry.notify(child);

        assert!(registry.parents_of[child.index() as usize].is_empty());
    }

    #[test]
    fn consume_prunes_edge_and_row_and_recycled_slot_starts_clean() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        registry.attach(set, child, 1).unwrap();

        registry.consume(child).unwrap();

        assert_eq!(waitset_of(&mut registry, set).child_count(), 0);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
        assert!(registry.parents_of[child.index() as usize].is_empty());

        for _ in 2..=(FUTURE_SLOTS as u16 - 2) {
            registry.register(Box::new(DummyFuture)).unwrap();
        }
        let fresh_set = register_waitset(&mut registry);
        let recycled = registry.register(Box::new(DummyFuture)).unwrap();
        assert_eq!(recycled.index(), child.index());
        assert_eq!(recycled.generation(), (child.generation() + 1) & 0x7FFF);

        registry.attach(fresh_set, recycled, 4).unwrap();
        registry.notify(recycled);
        expect_event(registry.take_pending(fresh_set), recycled, 4);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
    }

    #[test]
    fn notify_with_stale_generation_restores_occupant_row_without_recording() {
        let mut registry = FutureRegistry::new();
        let old_set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        registry.attach(old_set, child, 1).unwrap();
        registry.consume(child).unwrap();

        let fresh = registry.register(Box::new(DummyFuture)).unwrap();
        let fresh_set = register_waitset(&mut registry);
        registry.attach(fresh_set, fresh, 7).unwrap();

        let stale = FutureHandle::new(fresh.index(), (fresh.generation() + 1) & 0x7FFF);
        registry.notify(stale);

        assert!(waitset_of(&mut registry, fresh_set).has_child(fresh));
        assert!(!waitset_of(&mut registry, fresh_set).is_completed());
        assert!(matches!(registry.take_pending(old_set), WaitsetTake::Empty));
        assert_eq!(registry.parents_of[fresh.index() as usize].len(), 1);
    }

    #[test]
    fn take_pending_drops_events_whose_child_is_dead() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        registry.consume(child).unwrap();
        {
            let future = registry.borrow_mut(set).unwrap();
            let waitset = future.as_any_mut().downcast_mut::<WaitsetFuture>().unwrap();
            waitset.record(WaitEvent { child: child.pack() as u32, tag: 5 });
        }
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
    }

    #[test]
    fn detach_purges_both_sides_and_allows_reattach() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        registry.attach(set, child, 1).unwrap();

        assert_eq!(registry.detach(set, child), Ok(()));
        registry.notify(child);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
        assert_eq!(registry.detach(set, child), Err(AttachError::ChildInvalid));

        assert_eq!(registry.attach(set, child, 2), Ok(()));
        assert_eq!(registry.parents_of[child.index() as usize].len(), 1);
        registry.notify(child);
        expect_event(registry.take_pending(set), child, 2);
    }

    #[test]
    fn detach_rejects_absent_and_non_waitset_parents() {
        let mut registry = FutureRegistry::new();
        let plain = registry.register(Box::new(DummyFuture)).unwrap();
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        assert_eq!(registry.detach(plain, child), Err(AttachError::ParentNotFound));
        assert_eq!(
            registry.detach(FutureHandle::new(704, 9), child),
            Err(AttachError::ParentNotFound)
        );
    }

    #[test]
    fn drop_waitset_wakes_blocked_waiter_prunes_child_and_is_idempotent() {
        setup();

        let mut registry = services().future_registry.borrow_mut();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture)).unwrap();
        registry.attach(set, child, 1).unwrap();

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);
        registry.register_waiter(set, handle);

        registry.drop_waitset(set);

        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Gone));
        assert!(registry.parents_of[child.index() as usize].is_empty());
        registry.drop_waitset(set);
    }

    #[test]
    fn take_pending_on_non_waitset_or_absent_handle_is_gone() {
        let mut registry = FutureRegistry::new();
        let plain = registry.register(Box::new(DummyFuture)).unwrap();
        assert!(matches!(registry.take_pending(plain), WaitsetTake::Gone));
        assert!(matches!(
            registry.take_pending(FutureHandle::new(702, 9)),
            WaitsetTake::Gone
        ));
    }
}


