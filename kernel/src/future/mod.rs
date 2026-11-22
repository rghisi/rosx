pub(crate) mod waitset;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::any::Any;
use crate::collections::arena::{Error, GenerationalArena};
use crate::handle::FutureHandle;
use crate::irq_guard::IrqCell;
use crate::task::{GroupId, TaskHandle};
use crate::ports::driven::ForWakingTasks;
use self::waitset::{AttachError, ParentLink, WaitsetFuture, WaitsetTake, WAITSET_CHILD_CAPACITY};
use system::waitset::WaitEvent;

pub(crate) const FUTURE_SLOTS: usize = 1024;

static FUTURE_SECTION: IrqCell = IrqCell::new("future_registry");

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FutureOwner {
    Group(GroupId),
    Kernel,
    Free,
}

pub trait Future: Send + Sync {
    fn is_completed(&self) -> bool;

    fn owner(&self) -> TaskHandle;

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
    owner: TaskHandle,
}

impl TimeFuture {
    pub fn owned_by(owner: TaskHandle) -> TimeFuture {
        TimeFuture { completed: false, owner }
    }
}
impl Future for TimeFuture {
    fn is_completed(&self) -> bool {
        self.completed
    }

    fn owner(&self) -> TaskHandle {
        self.owner
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
    completed: bool,
    status: i32,
    owner: TaskHandle,
}

impl TaskCompletionFuture {
    pub fn owned_by(owner: TaskHandle) -> Self {
        Self {
            completed: false,
            status: system::exit::encode(system::exit::EXIT_REASON_CLEAN, 0),
            owner,
        }
    }

    pub(crate) fn stamp(&mut self, status: i32) {
        self.completed = true;
        self.status = status;
    }

    pub(crate) fn status(&self) -> i32 {
        self.status
    }
}

impl Future for TaskCompletionFuture {
    fn is_completed(&self) -> bool {
        self.completed
    }

    fn owner(&self) -> TaskHandle {
        self.owner
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

struct FutureEntry {
    future: Box<dyn Future + Send + Sync>,
    owner: FutureOwner,
    parents: Option<Vec<ParentLink>>,
}

impl FutureEntry {
    fn add_parent(&mut self, link: ParentLink) {
        self.parents.get_or_insert_with(Vec::new).push(link);
    }

    fn take_parents(&mut self) -> Option<Vec<ParentLink>> {
        self.parents.take()
    }

    fn set_parents(&mut self, kept: Vec<ParentLink>) {
        self.parents = if kept.is_empty() { None } else { Some(kept) };
    }
}

pub struct FutureRegistry {
    arena: GenerationalArena<FutureEntry>,
    waiters: BTreeMap<FutureHandle, Vec<TaskHandle>>,
    by_owner: BTreeMap<TaskHandle, Vec<FutureHandle>>,
    wake_controller: &'static dyn ForWakingTasks,
}

struct NoopWakeController;
impl ForWakingTasks for NoopWakeController {
    fn wake_tasks(&self, _handles: Vec<TaskHandle>) {}
}

static DEFAULT_WAKE_CONTROLLER: NoopWakeController = NoopWakeController;

impl Default for FutureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn is_waitset(future: &Box<dyn Future + Send + Sync>) -> bool {
    future.as_any().downcast_ref::<WaitsetFuture>().is_some()
}

fn as_waitset(future: &mut Box<dyn Future + Send + Sync>) -> Option<&mut WaitsetFuture> {
    future.as_any_mut().downcast_mut::<WaitsetFuture>()
}

impl FutureRegistry {
    pub fn new() -> Self {
        Self {
            arena: GenerationalArena::new(FUTURE_SLOTS),
            waiters: BTreeMap::new(),
            by_owner: BTreeMap::new(),
            wake_controller: &DEFAULT_WAKE_CONTROLLER,
        }
    }

    pub(crate) fn new_with_wake_controller(wake_controller: &'static dyn ForWakingTasks) -> Self {
        Self {
            arena: GenerationalArena::new(FUTURE_SLOTS),
            waiters: BTreeMap::new(),
            by_owner: BTreeMap::new(),
            wake_controller,
        }
    }

    pub fn register(
        &mut self,
        future: Box<dyn Future + Send + Sync>,
        owner: FutureOwner,
    ) -> Option<FutureHandle> {
        let task_owner = future.owner();
        let handle = self.arena.add(FutureEntry { future, owner, parents: None }).ok()?;
        self.by_owner.entry(task_owner).or_default().push(FutureHandle::from(handle));
        Some(FutureHandle::from(handle))
    }

    pub fn register_waiter(&mut self, future_handle: FutureHandle, task_handle: TaskHandle) {
        let _section = FUTURE_SECTION.acquire();
        self.waiters.entry(future_handle).or_default().push(task_handle);
    }

    pub fn get(&mut self, handle: FutureHandle) -> Option<bool> {
        if let Ok(entry) = self.arena.borrow_mut(handle.into()) {
            Some(entry.future.is_completed())
        } else {
            None
        }
    }

    pub fn consume(&mut self, handle: FutureHandle) -> Result<Box<dyn Future + Send + Sync>, Error> {
        let _section = FUTURE_SECTION.acquire();
        self.consume_inner(handle)
    }

    fn consume_inner(&mut self, handle: FutureHandle) -> Result<Box<dyn Future + Send + Sync>, Error> {
        let removed = self.arena.remove(handle.into())?;
        let owner = removed.future.owner();
        if let Some(handles) = self.by_owner.get_mut(&owner) {
            handles.retain(|live| *live != handle);
            if handles.is_empty() {
                self.by_owner.remove(&owner);
            }
        }
        for link in removed.parents.unwrap_or_default() {
            if let Ok(parent) = self.arena.borrow_mut(link.parent.into()) {
                if let Some(waitset) = as_waitset(&mut parent.future) {
                    waitset.remove_child(handle);
                }
            }
        }
        Ok(removed.future)
    }

    pub fn borrow_mut(&mut self, handle: FutureHandle) -> Result<&mut Box<dyn Future + Send + Sync>, Error> {
        let entry = self.arena.borrow_mut(handle.into())?;
        Ok(&mut entry.future)
    }

    pub fn notify(&mut self, handle: FutureHandle) {
        let _section = FUTURE_SECTION.acquire();
        self.notify_inner(handle);
    }

    pub fn notify_with_status(&mut self, handle: FutureHandle, status: i32) {
        let _section = FUTURE_SECTION.acquire();
        if let Ok(entry) = self.arena.borrow_mut(handle.into()) {
            if let Some(completion) = entry.future.as_any_mut().downcast_mut::<TaskCompletionFuture>() {
                completion.stamp(status);
            }
        }
        self.notify_inner(handle);
    }

    pub(crate) fn completion_owner(&mut self, handle: FutureHandle) -> Option<TaskHandle> {
        self.arena
            .borrow(handle.into())
            .ok()
            .and_then(|entry| entry.future.as_any().downcast_ref::<TaskCompletionFuture>())
            .map(|completion| completion.owner())
    }

    pub(crate) fn discard_waiter(&mut self, task: TaskHandle) {
        let _section = FUTURE_SECTION.acquire();
        for waiters in self.waiters.values_mut() {
            waiters.retain(|waiter| *waiter != task);
        }
        self.waiters.retain(|_, waiters| !waiters.is_empty());
    }

    pub(crate) fn discard_owned(&mut self, task: TaskHandle, spare: Option<FutureHandle>) -> Vec<FutureHandle> {
        let _section = FUTURE_SECTION.acquire();
        let owned = self.by_owner.remove(&task).unwrap_or_default();
        let mut discarded = Vec::new();
        for handle in owned {
            if Some(handle) == spare {
                continue;
            }
            if self.consume_inner(handle).is_ok() {
                self.waiters.remove(&handle);
                discarded.push(handle);
            }
        }
        if let Some(spare) = spare {
            if self.is_live(spare) {
                self.by_owner.entry(task).or_default().push(spare);
            }
        }
        discarded
    }

    fn notify_inner(&mut self, handle: FutureHandle) {
        let waiters = self.waiters.remove(&handle).unwrap_or_default();
        self.wake_controller.wake_tasks(waiters);
        self.cascade_to_waitsets(handle);
    }

    pub(crate) fn is_waitset(&mut self, handle: FutureHandle) -> bool {
        match self.arena.borrow(handle.into()) {
            Ok(entry) => is_waitset(&entry.future),
            Err(_) => false,
        }
    }

    pub(crate) fn attach(&mut self, set: FutureHandle, child: FutureHandle, tag: u32) -> Result<(), AttachError> {
        let _section = FUTURE_SECTION.acquire();
        let child_completed = match self.arena.borrow(child.into()) {
            Ok(entry) => {
                if is_waitset(&entry.future) {
                    return Err(AttachError::NestingForbidden);
                }
                entry.future.is_completed()
            }
            Err(_) => return Err(AttachError::ChildInvalid),
        };
        {
            let parent = match self.arena.borrow_mut(set.into()) {
                Ok(entry) => match as_waitset(&mut entry.future) {
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
        if let Ok(entry) = self.arena.borrow_mut(child.into()) {
            entry.add_parent(ParentLink { parent: set, tag });
        }
        if child_completed {
            self.notify_inner(set);
        }
        Ok(())
    }

    pub(crate) fn detach(&mut self, set: FutureHandle, child: FutureHandle) -> Result<(), AttachError> {
        let edge_removed = match self.arena.borrow_mut(set.into()) {
            Ok(entry) => match as_waitset(&mut entry.future) {
                Some(waitset) => waitset.remove_child(child),
                None => return Err(AttachError::ParentNotFound),
            },
            Err(_) => return Err(AttachError::ParentNotFound),
        };
        if !edge_removed {
            return Err(AttachError::ChildInvalid);
        }
        if let Ok(entry) = self.arena.borrow_mut(child.into()) {
            if let Some(parents) = &mut entry.parents {
                parents.retain(|link| link.parent != set);
                if parents.is_empty() {
                    entry.parents = None;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn take_pending(&mut self, set: FutureHandle) -> WaitsetTake {
        loop {
            let event = match self.arena.borrow_mut(set.into()) {
                Ok(entry) => match as_waitset(&mut entry.future) {
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
        let _section = FUTURE_SECTION.acquire();
        let children = match self.arena.borrow_mut(set.into()) {
            Ok(entry) => match as_waitset(&mut entry.future) {
                Some(waitset) => waitset.child_handles(),
                None => return,
            },
            Err(_) => return,
        };
        for child in children {
            let _ = self.detach(set, child);
        }
        self.notify_inner(set);
        let _ = self.consume_inner(set);
    }

    pub(crate) fn reclaim_group(&mut self, group: GroupId) {
        let _section = FUTURE_SECTION.acquire();
        let owner = FutureOwner::Group(group);
        let mut abandoned = Vec::new();
        self.arena.visit_handles(|handle, entry| {
            if entry.owner == owner {
                abandoned.push(FutureHandle::from(handle));
            }
        });
        for handle in abandoned {
            self.notify_inner(handle);
            let _ = self.consume_inner(handle);
        }
    }

    pub(crate) fn is_live(&mut self, handle: FutureHandle) -> bool {
        self.arena.borrow(handle.into()).is_ok()
    }

    fn cascade_to_waitsets(&mut self, handle: FutureHandle) {
        let row = match self.arena.borrow_mut(handle.into()) {
            Ok(entry) => match entry.take_parents() {
                Some(row) => row,
                None => return,
            },
            Err(_) => return,
        };
        let child_token = handle.pack() as u32;
        let mut kept = Vec::with_capacity(row.len());
        let mut awakened = Vec::new();
        for link in row {
            let recorded = match self.arena.borrow_mut(link.parent.into()) {
                Ok(entry) => match as_waitset(&mut entry.future) {
                    Some(waitset) => Some(waitset.record(WaitEvent { child: child_token, tag: link.tag })),
                    None => None,
                },
                Err(_) => None,
            };
            match recorded {
                None => continue,
                Some(became_signalled) => {
                    if became_signalled {
                        awakened.push(link.parent);
                    }
                    kept.push(link);
                }
            }
        }
        if let Ok(entry) = self.arena.borrow_mut(handle.into()) {
            entry.set_parents(kept);
        }
        for parent in awakened {
            self.notify_inner(parent);
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::arena::PAGE_SLOTS;
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

    const GHOST_OWNER: TaskHandle = TaskHandle::new(u16::MAX, 0);

    struct DummyFuture(TaskHandle);
    impl Future for DummyFuture {
        fn is_completed(&self) -> bool {
            false
        }
        fn owner(&self) -> TaskHandle {
            self.0
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

        let future_handle = services().future_registry.borrow_mut().register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
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

        let future_handle = services().future_registry.borrow_mut().register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        services().future_registry.borrow_mut().register_waiter(future_handle, handle);

        // First notify moves task to Ready
        services().future_registry.borrow_mut().notify(future_handle);
        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);

        // Second notify does nothing (no waiters left)
        services().future_registry.borrow_mut().notify(future_handle);
        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
    }

    #[test]
    fn task_completion_future_starts_incomplete_with_clean_zero() {
        setup();
        let future = TaskCompletionFuture::owned_by(TaskHandle::new(1, 0));

        assert!(!future.is_completed());
        assert_eq!(future.status(), system::exit::clean_status(0).unwrap());
    }

    #[test]
    fn task_completion_state_is_decoupled_from_task_state() {
        setup();
        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();

        let completion = Box::new(TaskCompletionFuture::owned_by(TaskHandle::new(1, 0)));
        let future_handle = services().future_registry.borrow_mut().register(completion, FutureOwner::Kernel).unwrap();

        services().task_manager.borrow_mut().set_state(handle, TaskState::Terminated);
        assert_eq!(services().future_registry.borrow_mut().get(future_handle), Some(false));

        let status = system::exit::clean_status(7).unwrap();
        services().future_registry.borrow_mut().notify_with_status(future_handle, status);
        assert_eq!(services().future_registry.borrow_mut().get(future_handle), Some(true));
    }

    #[test]
    fn stamped_completion_survives_task_removal_and_slot_recycling() {
        setup();
        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();

        let completion = Box::new(TaskCompletionFuture::owned_by(TaskHandle::new(1, 0)));
        let future_handle = services().future_registry.borrow_mut().register(completion, FutureOwner::Kernel).unwrap();

        let status = system::exit::clean_status(7).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Terminated);
        services().future_registry.borrow_mut().notify_with_status(future_handle, status);
        services().task_manager.borrow_mut().remove_task(handle);

        let occupier = services().task_manager.borrow_mut().add_task(Task::new("T", 0x1000, 0)).unwrap();
        assert_eq!(services().task_manager.borrow().get_state(occupier), TaskState::Created);

        let taken = services().future_registry.borrow_mut().consume(future_handle).unwrap();
        let completion = taken.into_any().downcast::<TaskCompletionFuture>().unwrap();
        assert!(completion.is_completed());
        assert_eq!(completion.status(), status);
    }

    #[test]
    fn notify_with_status_stamps_task_completion_future_and_wakes_waiters() {
        setup();
        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);

        let completion = Box::new(TaskCompletionFuture::owned_by(TaskHandle::new(1, 0)));
        let future_handle = services().future_registry.borrow_mut().register(completion, FutureOwner::Kernel).unwrap();
        services().future_registry.borrow_mut().register_waiter(future_handle, handle);

        let status = system::exit::encode(system::exit::EXIT_REASON_PANICKED, 0);
        services().future_registry.borrow_mut().notify_with_status(future_handle, status);

        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
        let future = services().future_registry.borrow_mut().borrow_mut(future_handle).unwrap();
        let completion = future.as_any_mut().downcast_mut::<TaskCompletionFuture>().unwrap();
        assert!(completion.is_completed());
        assert_eq!(completion.status(), status);
    }

    #[test]
    fn notify_with_status_wakes_non_task_future_waiters_and_carries_no_status() {
        setup();
        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);

        let future_handle = services().future_registry.borrow_mut().register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        services().future_registry.borrow_mut().register_waiter(future_handle, handle);

        services().future_registry.borrow_mut().notify_with_status(future_handle, 7);

        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
    }

    #[test]
    fn consumed_task_completion_future_carries_the_stamped_status() {
        setup();

        let completion = Box::new(TaskCompletionFuture::owned_by(TaskHandle::new(1, 0)));
        let future_handle = services().future_registry.borrow_mut().register(completion, FutureOwner::Kernel).unwrap();

        let status = system::exit::clean_status(3).unwrap();
        services().future_registry.borrow_mut().notify_with_status(future_handle, status);

        let taken = services().future_registry.borrow_mut().consume(future_handle).unwrap();
        let completion = taken.into_any().downcast::<TaskCompletionFuture>().unwrap();
        assert!(completion.is_completed());
        assert_eq!(completion.status(), status);
    }

    #[test]
    fn notify_with_status_on_unknown_handle_is_inert() {
        setup();
        let registry = &services().future_registry;
        let completion = Box::new(TaskCompletionFuture::owned_by(TaskHandle::new(1, 0)));
        let live_handle = registry.borrow_mut().register(completion, FutureOwner::Kernel).unwrap();
        let _ = registry.borrow_mut().consume(live_handle).unwrap();

        registry.borrow_mut().notify_with_status(live_handle, 5);
    }

    // === Ownership stamping tests ===

    fn owner_of(registry: &FutureRegistry, handle: FutureHandle) -> FutureOwner {
        registry
            .arena
            .borrow(handle.into())
            .map(|entry| entry.owner)
            .unwrap_or(FutureOwner::Free)
    }

    fn parent_links(registry: &FutureRegistry, handle: FutureHandle) -> usize {
        registry
            .arena
            .borrow(handle.into())
            .ok()
            .and_then(|entry| entry.parents.as_ref().map(|links| links.len()))
            .unwrap_or(0)
    }

    #[test]
    fn register_stamps_the_owner_and_consume_clears_it() {
        let mut registry = FutureRegistry::new();
        let group = GroupId::allocate();
        let handle = registry
            .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Group(group))
            .unwrap();
        assert_eq!(owner_of(&registry, handle), FutureOwner::Group(group));

        registry.consume(handle).unwrap();
        assert_eq!(owner_of(&registry, handle), FutureOwner::Free);
    }

    #[test]
    fn a_recycled_slot_carries_the_new_owner() {
        let mut registry = FutureRegistry::new();
        let kernel_slot = registry
            .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel)
            .unwrap();
        assert_eq!(owner_of(&registry, kernel_slot), FutureOwner::Kernel);
        registry.consume(kernel_slot).unwrap();

        for _ in 1..PAGE_SLOTS {
            registry
                .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel)
                .unwrap();
        }
        let group = GroupId::allocate();
        let recycled = registry
            .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Group(group))
            .unwrap();
        assert_eq!(recycled.index(), kernel_slot.index());
        assert_eq!(owner_of(&registry, recycled), FutureOwner::Group(group));
    }

    // === Reclaim tests ===

    #[test]
    fn reclaim_group_frees_only_the_owners_slots() {
        let mut registry = FutureRegistry::new();
        let doomed = GroupId::allocate();
        let neighbor = GroupId::allocate();
        let owned = registry
            .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Group(doomed))
            .unwrap();
        let other = registry
            .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Group(neighbor))
            .unwrap();
        let kernel_slot = registry
            .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel)
            .unwrap();

        registry.reclaim_group(doomed);

        assert_eq!(registry.get(owned), None);
        assert_eq!(owner_of(&registry, owned), FutureOwner::Free);
        assert_eq!(registry.get(other), Some(false));
        assert_eq!(registry.get(kernel_slot), Some(false));

        registry.reclaim_group(GroupId::allocate());
        assert_eq!(registry.get(other), Some(false));
        assert_eq!(registry.get(kernel_slot), Some(false));
    }

    #[test]
    fn reclaim_wakes_a_parked_cross_group_waiter_which_consumes_to_an_honest_error() {
        setup();
        let doomed = GroupId::allocate();

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);

        let future_handle = services()
            .future_registry
            .borrow_mut()
            .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Group(doomed))
            .unwrap();
        services().future_registry.borrow_mut().register_waiter(future_handle, handle);

        services().future_registry.borrow_mut().reclaim_group(doomed);

        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
        let consumed = services().future_registry.borrow_mut().consume(future_handle);
        assert!(matches!(consumed, Err(Error::NotFound)));
    }

    #[test]
    fn reclaim_purges_the_childs_parent_links_and_leaves_the_waitset_alive() {
        let mut registry = FutureRegistry::new();
        let doomed = GroupId::allocate();
        let set_owner = GroupId::allocate();
        let set = registry
            .register(Box::new(WaitsetFuture::owned_by(GHOST_OWNER)), FutureOwner::Group(set_owner))
            .unwrap();
        let child = registry
            .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Group(doomed))
            .unwrap();
        registry.attach(set, child, 1).unwrap();

        registry.reclaim_group(doomed);

        assert_eq!(registry.get(child), None);
        assert_eq!(parent_links(&registry, child), 0);
        assert!(registry.is_waitset(set));
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
    }

    #[test]
    fn a_reclaimed_slot_recycled_by_the_kernel_is_immune_to_the_dead_groups_handles() {
        let mut registry = FutureRegistry::new();
        let doomed = GroupId::allocate();
        let doomed_handle = registry
            .register(Box::new(TaskCompletionFuture::owned_by(TaskHandle::new(1, 0))), FutureOwner::Group(doomed))
            .unwrap();

        registry.reclaim_group(doomed);

        for _ in 1..PAGE_SLOTS {
            registry
                .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel)
                .unwrap();
        }
        let recycled = registry
            .register(Box::new(TaskCompletionFuture::owned_by(TaskHandle::new(1, 0))), FutureOwner::Kernel)
            .unwrap();
        assert_eq!(recycled.index(), doomed_handle.index());

        registry.notify_with_status(doomed_handle, system::exit::clean_status(9).unwrap());

        assert_eq!(registry.get(recycled), Some(false));
        assert!(registry.consume(doomed_handle).is_err());
        assert_eq!(owner_of(&registry, recycled), FutureOwner::Kernel);
    }

    #[test]
    fn kernel_owned_slots_survive_every_sweep() {
        let mut registry = FutureRegistry::new();
        let kernel_slot = registry
            .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel)
            .unwrap();
        for _ in 0..5 {
            let group = GroupId::allocate();
            let slot = registry
                .register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Group(group))
                .unwrap();
            registry.reclaim_group(group);
            assert_eq!(registry.get(slot), None);
            assert_eq!(registry.get(kernel_slot), Some(false));
        }
    }

    #[test]
    fn a_child_group_sweep_spares_the_parents_completion_future() {
        let mut registry = FutureRegistry::new();
        let parent_group = GroupId::allocate();
        let child_group = GroupId::allocate();
        let completion = registry
            .register(Box::new(TaskCompletionFuture::owned_by(TaskHandle::new(1, 0))), FutureOwner::Group(parent_group))
            .unwrap();
        let child_slot = registry
            .register(Box::new(TimeFuture::owned_by(GHOST_OWNER)), FutureOwner::Group(child_group))
            .unwrap();

        registry.reclaim_group(child_group);

        assert_eq!(registry.get(child_slot), None);
        assert_eq!(registry.get(completion), Some(false));
        let status = system::exit::clean_status(3).unwrap();
        registry.notify_with_status(completion, status);
        let taken = registry.consume(completion).unwrap();
        let completion = taken.into_any().downcast::<TaskCompletionFuture>().unwrap();
        assert_eq!(completion.status(), status);
    }

    // === TimeFuture tests ===

    #[test]
    fn time_future_starts_not_completed() {
        let future = TimeFuture::owned_by(GHOST_OWNER);
        assert!(!future.is_completed());
    }

    #[test]
    fn time_future_complete_flips_flag() {
        let mut future = TimeFuture::owned_by(GHOST_OWNER);
        future.complete();
        assert!(future.is_completed());
    }

    #[test]
    fn time_future_multiple_completes_are_idempotent() {
        let mut future = TimeFuture::owned_by(GHOST_OWNER);
        future.complete();
        future.complete();
        future.complete();
        assert!(future.is_completed());
    }

    #[test]
    fn future_registry_can_complete_time_future_via_trait() {
        setup();

        let mut registry = FutureRegistry::new();
        let time_future = Box::new(TimeFuture::owned_by(GHOST_OWNER));
        let handle = registry.register(time_future, FutureOwner::Kernel).unwrap();

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
        let dummy = Box::new(DummyFuture(GHOST_OWNER));
        let handle = registry.register(dummy, FutureOwner::Kernel).unwrap();

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
        fn owner(&self) -> TaskHandle {
            GHOST_OWNER
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
        registry.register(Box::new(WaitsetFuture::owned_by(GHOST_OWNER)), FutureOwner::Kernel).unwrap()
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
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        let absent = FutureHandle::new(700, 9);
        assert_eq!(registry.attach(absent, child, 0), Err(AttachError::ParentNotFound));
        assert_eq!(parent_links(&registry, child), 0);
    }

    #[test]
    fn attach_rejects_parent_that_is_not_a_waitset() {
        let mut registry = FutureRegistry::new();
        let plain = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        assert_eq!(registry.attach(plain, child, 0), Err(AttachError::ParentNotFound));
        assert_eq!(parent_links(&registry, child), 0);
    }

    #[test]
    fn attach_rejects_absent_child() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let absent = FutureHandle::new(701, 9);
        assert_eq!(registry.attach(set, absent, 0), Err(AttachError::ChildInvalid));
        assert_eq!(parent_links(&registry, absent), 0);
        assert_eq!(waitset_of(&mut registry, set).child_count(), 0);
    }

    #[test]
    fn attach_rejects_waitset_child_and_self_attach() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let other_set = register_waitset(&mut registry);
        assert_eq!(registry.attach(set, other_set, 0), Err(AttachError::NestingForbidden));
        assert_eq!(registry.attach(set, set, 7), Err(AttachError::NestingForbidden));
        assert_eq!(parent_links(&registry, other_set), 0);
        assert_eq!(parent_links(&registry, set), 0);
    }

    #[test]
    fn attach_rejects_duplicate_child() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        assert_eq!(registry.attach(set, child, 1), Ok(()));
        assert_eq!(registry.attach(set, child, 2), Err(AttachError::ChildInvalid));
        assert_eq!(parent_links(&registry, child), 1);
        assert_eq!(waitset_of(&mut registry, set).child_count(), 1);
    }

    #[test]
    fn attach_rejects_child_beyond_capacity() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        for _ in 0..WAITSET_CHILD_CAPACITY {
            let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
            assert_eq!(registry.attach(set, child, 0), Ok(()));
        }
        let overflow = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        assert_eq!(registry.attach(set, overflow, 0), Err(AttachError::Full));
        assert_eq!(parent_links(&registry, overflow), 0);
    }

    #[test]
    fn attach_latches_already_completed_child() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let done = registry.register(Box::new(DummyCompletedFuture), FutureOwner::Kernel).unwrap();
        assert_eq!(registry.attach(set, done, 42), Ok(()));
        assert_eq!(registry.get(set), Some(true));
        expect_event(registry.take_pending(set), done, 42);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
    }

    #[test]
    fn is_waitset_distinguishes_waitsets_plain_futures_and_absent_handles() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let plain = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        assert!(registry.is_waitset(set));
        assert!(!registry.is_waitset(plain));
        assert!(!registry.is_waitset(FutureHandle::new(703, 9)));
    }

    #[test]
    fn notify_child_records_event_and_wakes_parent_waiter() {
        setup();

        let mut registry = services().future_registry.borrow_mut();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
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
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
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
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        registry.attach(set, child, 3).unwrap();
        registry.consume(set).unwrap();

        registry.notify(child);

        assert_eq!(parent_links(&registry, child), 0);
    }

    #[test]
    fn consume_prunes_edge_and_row_and_recycled_slot_starts_clean() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        registry.attach(set, child, 1).unwrap();

        registry.consume(child).unwrap();

        assert_eq!(waitset_of(&mut registry, set).child_count(), 0);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
        assert_eq!(parent_links(&registry, child), 0);

        for _ in 2..PAGE_SLOTS {
            registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        }
        let recycled = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        assert_eq!(recycled.index(), child.index());
        assert_eq!(recycled.generation(), (child.generation() + 1) & 0x7FFF);
        let fresh_set = register_waitset(&mut registry);

        registry.attach(fresh_set, recycled, 4).unwrap();
        registry.notify(recycled);
        expect_event(registry.take_pending(fresh_set), recycled, 4);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
    }

    #[test]
    fn notify_with_stale_generation_restores_occupant_row_without_recording() {
        let mut registry = FutureRegistry::new();
        let old_set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        registry.attach(old_set, child, 1).unwrap();
        registry.consume(child).unwrap();

        let fresh = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        let fresh_set = register_waitset(&mut registry);
        registry.attach(fresh_set, fresh, 7).unwrap();

        let stale = FutureHandle::new(fresh.index(), (fresh.generation() + 1) & 0x7FFF);
        registry.notify(stale);

        assert!(waitset_of(&mut registry, fresh_set).has_child(fresh));
        assert!(!waitset_of(&mut registry, fresh_set).is_completed());
        assert!(matches!(registry.take_pending(old_set), WaitsetTake::Empty));
        assert_eq!(parent_links(&registry, fresh), 1);
    }

    #[test]
    fn take_pending_drops_events_whose_child_is_dead() {
        let mut registry = FutureRegistry::new();
        let set = register_waitset(&mut registry);
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
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
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        registry.attach(set, child, 1).unwrap();

        assert_eq!(registry.detach(set, child), Ok(()));
        registry.notify(child);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Empty));
        assert_eq!(registry.detach(set, child), Err(AttachError::ChildInvalid));

        assert_eq!(registry.attach(set, child, 2), Ok(()));
        assert_eq!(parent_links(&registry, child), 1);
        registry.notify(child);
        expect_event(registry.take_pending(set), child, 2);
    }

    #[test]
    fn detach_rejects_absent_and_non_waitset_parents() {
        let mut registry = FutureRegistry::new();
        let plain = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
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
        let child = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        registry.attach(set, child, 1).unwrap();

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);
        registry.register_waiter(set, handle);

        registry.drop_waitset(set);

        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
        assert!(matches!(registry.take_pending(set), WaitsetTake::Gone));
        assert_eq!(parent_links(&registry, child), 0);
        registry.drop_waitset(set);
    }

    #[test]
    fn take_pending_on_non_waitset_or_absent_handle_is_gone() {
        let mut registry = FutureRegistry::new();
        let plain = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        assert!(matches!(registry.take_pending(plain), WaitsetTake::Gone));
        assert!(matches!(
            registry.take_pending(FutureHandle::new(702, 9)),
            WaitsetTake::Gone
        ));
    }

    #[test]
    fn completion_owner_reads_the_stamped_task_and_refuses_other_futures() {
        let mut registry = FutureRegistry::new();
        let owner = TaskHandle::new(7, 2);
        let completion = registry.register(Box::new(TaskCompletionFuture::owned_by(owner)), FutureOwner::Kernel).unwrap();
        assert_eq!(registry.completion_owner(completion), Some(owner));

        let plain = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        assert_eq!(registry.completion_owner(plain), None);
        assert_eq!(registry.completion_owner(FutureHandle::new(999, 3)), None);
    }

    #[test]
    fn discard_waiter_evicts_the_task_from_every_waiter_list() {
        let mut registry = FutureRegistry::new();
        let ghost = TaskHandle::new(40, 0);
        let first = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        let second = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        let live = TaskHandle::new(41, 0);
        registry.register_waiter(first, ghost);
        registry.register_waiter(first, live);
        registry.register_waiter(second, ghost);

        registry.discard_waiter(ghost);

        assert_eq!(registry.waiters.get(&first), Some(&alloc::vec![live]));
        assert!(!registry.waiters.contains_key(&second));
    }

    #[test]
    fn discarded_waiter_is_not_woken_by_a_later_notify() {
        let mut registry = FutureRegistry::new();
        let ghost = TaskHandle::new(42, 0);
        let future = registry.register(Box::new(DummyFuture(GHOST_OWNER)), FutureOwner::Kernel).unwrap();
        registry.register_waiter(future, ghost);
        registry.discard_waiter(ghost);

        registry.notify(future);

        assert!(!registry.waiters.contains_key(&future));
    }

    #[test]
    fn register_indexes_every_future_by_owner() {
        let mut registry = FutureRegistry::new();
        let victim = TaskHandle::new(50, 0);
        let first = registry
            .register(Box::new(TimeFuture::owned_by(victim)), FutureOwner::Kernel)
            .unwrap();
        let second = registry
            .register(Box::new(WaitsetFuture::owned_by(victim)), FutureOwner::Kernel)
            .unwrap();

        assert_eq!(registry.by_owner.get(&victim), Some(&alloc::vec![first, second]));
    }

    #[test]
    fn consume_prunes_the_owner_index_and_empty_rows_disappear() {
        let mut registry = FutureRegistry::new();
        let victim = TaskHandle::new(51, 0);
        let first = registry
            .register(Box::new(DummyFuture(victim)), FutureOwner::Kernel)
            .unwrap();
        let second = registry
            .register(Box::new(DummyFuture(victim)), FutureOwner::Kernel)
            .unwrap();

        registry.consume(first).unwrap();

        assert_eq!(registry.by_owner.get(&victim), Some(&alloc::vec![second]));
        registry.consume(second).unwrap();
        assert!(!registry.by_owner.contains_key(&victim));
    }

    #[test]
    fn discard_owned_frees_every_future_of_the_task_and_spares_the_named_completion() {
        let mut registry = FutureRegistry::new();
        let victim = TaskHandle::new(52, 0);
        let bystander = TaskHandle::new(53, 0);
        let sleeper = registry
            .register(Box::new(TimeFuture::owned_by(victim)), FutureOwner::Kernel)
            .unwrap();
        let set = registry
            .register(Box::new(WaitsetFuture::owned_by(victim)), FutureOwner::Kernel)
            .unwrap();
        let completion = registry
            .register(Box::new(TaskCompletionFuture::owned_by(victim)), FutureOwner::Kernel)
            .unwrap();
        let foreign = registry
            .register(Box::new(DummyFuture(bystander)), FutureOwner::Kernel)
            .unwrap();

        let mut discarded = registry.discard_owned(victim, Some(completion));
        discarded.sort_by_key(|handle| handle.index());

        let mut expected = alloc::vec![sleeper, set];
        expected.sort_by_key(|handle| handle.index());
        assert_eq!(discarded, expected);
        assert_eq!(registry.get(sleeper), None);
        assert_eq!(registry.get(set), None);
        assert_eq!(registry.get(completion), Some(false));
        assert_eq!(registry.get(foreign), Some(false));
        assert_eq!(registry.by_owner.get(&victim), Some(&alloc::vec![completion]));
    }

    #[test]
    fn discard_owned_detaches_the_victims_children_from_foreign_waitsets() {
        let mut registry = FutureRegistry::new();
        let victim = TaskHandle::new(54, 0);
        let foreign_set = registry
            .register(Box::new(WaitsetFuture::owned_by(GHOST_OWNER)), FutureOwner::Kernel)
            .unwrap();
        let child = registry
            .register(Box::new(DummyFuture(victim)), FutureOwner::Kernel)
            .unwrap();
        registry.attach(foreign_set, child, 3).unwrap();

        let discarded = registry.discard_owned(victim, None);

        assert_eq!(discarded, alloc::vec![child]);
        assert_eq!(waitset_of(&mut registry, foreign_set).child_count(), 0);
        assert_eq!(parent_links(&registry, child), 0);
    }

    #[test]
    fn discard_owned_releases_the_parked_waiter_entry_of_each_freed_future() {
        let mut registry = FutureRegistry::new();
        let victim = TaskHandle::new(55, 0);
        let parked = registry
            .register(Box::new(DummyFuture(victim)), FutureOwner::Kernel)
            .unwrap();
        registry.register_waiter(parked, victim);

        registry.discard_owned(victim, None);

        assert!(!registry.waiters.contains_key(&parked));
    }

    #[test]
    fn a_spared_completion_consumed_after_the_sweep_leaves_no_owner_row() {
        let mut registry = FutureRegistry::new();
        let victim = TaskHandle::new(56, 0);
        let completion = registry
            .register(Box::new(TaskCompletionFuture::owned_by(victim)), FutureOwner::Kernel)
            .unwrap();

        registry.discard_owned(victim, Some(completion));
        registry.consume(completion).unwrap();

        assert!(!registry.by_owner.contains_key(&victim));
        assert_eq!(registry.discard_owned(victim, None), alloc::vec![]);
    }
}


