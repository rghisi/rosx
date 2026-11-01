use crate::collections::arena::GenerationalArena;
use crate::task::TaskState::Terminated;
use crate::task::{GroupId, SharedTask, Task, TaskHandle, TaskState, YieldReason};
use core::ptr::null_mut;
use crate::handle::FutureHandle;

pub(crate) struct TaskManager {
    tasks: GenerationalArena<SharedTask, 256>,
}

#[derive(Debug)]
pub(crate) enum Error {
    TaskCannotBeAdded,
    TaskNotFound,
}

impl TaskManager {
    pub(crate) fn new() -> Self {
        TaskManager {
            tasks: GenerationalArena::new(),
        }
    }

    pub(crate) fn add_task(&mut self, task: SharedTask) -> Result<TaskHandle, Error> {
        match self.tasks.add(task) {
            Ok(handle) => Ok(TaskHandle::from(handle)),
            Err(_) => Err(Error::TaskCannotBeAdded),
        }
    }

    pub(crate) fn remove_task(&mut self, handle: TaskHandle) {
        let _ = self.tasks.remove(handle.into());
    }
    

    pub(crate) fn borrow_task_mut(&mut self, handle: TaskHandle) -> Result<&mut Task, Error> {
        match self.tasks.borrow_mut(handle.into()) {
            Ok(task) => Ok(task),
            Err(_) => Err(Error::TaskNotFound),
        }
    }

    pub(crate) fn get_state(&self, handle: TaskHandle) -> TaskState {
        match self.tasks.borrow(handle.into()) {
            Ok(task) => task.state(),
            Err(_) => Terminated,
        }
    }

    pub(crate) fn set_state(&mut self, handle: TaskHandle, state: TaskState) {
        if let Ok(task) = self.tasks.borrow_mut(handle.into()) {
            task.set_state(state)
        }
    }

    pub(crate) fn get_group(&self, handle: TaskHandle) -> Option<GroupId> {
        self.tasks.borrow(handle.into()).ok().map(|task| task.group())
    }

    pub(crate) fn group_has_live_members(&self, group: GroupId) -> bool {
        self.tasks
            .any(|task| task.group() == group && task.state() != Terminated)
    }

    pub(crate) fn set_yield_reason(&mut self, handle: TaskHandle, reason: YieldReason) {
        if let Ok(task) = self.tasks.borrow_mut(handle.into()) {
            task.set_yield_reason(reason)
        }
    }

    pub(crate) fn get_yield_reason(&self, handle: TaskHandle) -> Option<YieldReason> {
        match self.tasks.borrow(handle.into()) {
            Ok(task) => task.yield_reason(),
            Err(_) => None,
        }
    }

    pub(crate) fn get_task_stack_pointer(&self, handle: TaskHandle) -> usize {
        match self.tasks.borrow(handle.into()) {
            Ok(task) => task.stack_pointer(),
            Err(_) => 0,
        }
    }

    pub(crate) fn get_task_stack_pointer_ref(&mut self, handle: TaskHandle) -> *mut usize {
        match self.tasks.borrow_mut(handle.into()) {
            Ok(task) => task.stack_pointer_mut(),
            Err(_) => null_mut(),
        }
    }

    pub(crate) fn get_completion_future(&self, handle: TaskHandle) -> Option<FutureHandle> {
        match self.tasks.borrow(handle.into()) {
            Ok(task) => task.completion_future(),
            Err(_) => None,
        }
    }

    pub(crate) fn set_completion_future(&mut self, handle: TaskHandle, future_handle: FutureHandle) {
        if let Ok(task) = self.tasks.borrow_mut(handle.into()) {
            task.set_completion_future(future_handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use crate::task::new_entrypoint_task;
    use crate::collections::arena::GenerationalArena;

    fn make_future_handle() -> FutureHandle {
        let mut arena: GenerationalArena<u8, 1> = GenerationalArena::new();
        FutureHandle::from(arena.add(0u8).unwrap())
    }

    #[test]
    fn completion_future_is_none_by_default() {
        let mut manager = TaskManager::new();
        let task = Task::new("test", 0, 0);
        let handle = manager.add_task(task).unwrap();
        assert!(manager.get_completion_future(handle).is_none());
    }

    #[test]
    fn set_completion_future_persists() {
        let mut manager = TaskManager::new();
        let task = Task::new("test", 0, 0);
        let handle = manager.add_task(task).unwrap();
        let fh = make_future_handle();
        manager.set_completion_future(handle, fh);
        assert_eq!(manager.get_completion_future(handle), Some(fh));
    }

    #[test]
    fn fresh_tasks_belong_to_distinct_groups() {
        let mut manager = TaskManager::new();
        let first = manager.add_task(Task::new("first", 0, 0)).unwrap();
        let second = manager.add_task(Task::new("second", 0, 0)).unwrap();
        assert_ne!(manager.get_group(first), manager.get_group(second));
    }

    #[test]
    fn spawned_task_inherits_caller_group() {
        let mut manager = TaskManager::new();
        let caller = manager.add_task(Task::new("caller", 0, 0)).unwrap();
        let caller_group = manager.get_group(caller).unwrap();
        let spawned = manager.add_task(new_entrypoint_task(0, caller_group)).unwrap();
        assert_eq!(manager.get_group(spawned), Some(caller_group));
    }

    #[test]
    fn group_stays_live_until_its_last_member_terminates() {
        let mut manager = TaskManager::new();
        let caller = manager.add_task(Task::new("caller", 0, 0)).unwrap();
        let group = manager.get_group(caller).unwrap();
        let sibling = manager.add_task(new_entrypoint_task(0, group)).unwrap();
        assert!(manager.group_has_live_members(group));
        manager.set_state(caller, Terminated);
        assert!(manager.group_has_live_members(group));
        manager.set_state(sibling, Terminated);
        assert!(!manager.group_has_live_members(group));
    }

    #[test]
    fn terminating_one_group_leaves_other_groups_live() {
        let mut manager = TaskManager::new();
        let first = manager.add_task(Task::new("first", 0, 0)).unwrap();
        let second = manager.add_task(Task::new("second", 0, 0)).unwrap();
        let first_group = manager.get_group(first).unwrap();
        let second_group = manager.get_group(second).unwrap();
        manager.set_state(first, Terminated);
        assert!(!manager.group_has_live_members(first_group));
        assert!(manager.group_has_live_members(second_group));
    }

    #[test]
    fn removed_task_has_no_group() {
        let mut manager = TaskManager::new();
        let handle = manager.add_task(Task::new("gone", 0, 0)).unwrap();
        manager.remove_task(handle);
        assert_eq!(manager.get_group(handle), None);
    }
}
