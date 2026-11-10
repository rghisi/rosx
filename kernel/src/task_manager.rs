use crate::collections::arena::GenerationalArena;
use crate::task::TaskState::Terminated;
use crate::task::{GroupId, SharedTask, Task, TaskHandle, TaskState, YieldReason, TASK_NAME_BYTES};
use crate::task_stack::{StackObservation, STACK_WORDS};
use alloc::vec::Vec;
use core::ptr::null_mut;
use crate::handle::FutureHandle;

pub(crate) struct ProcessSnapshot {
    pub group: usize,
    pub state: TaskState,
    pub thread_count: u16,
    pub name: [u8; TASK_NAME_BYTES],
    pub name_len: usize,
}

fn liveness(state: TaskState) -> u8 {
    match state {
        TaskState::Running => 3,
        TaskState::Ready => 2,
        TaskState::Blocked => 1,
        _ => 0,
    }
}

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

    pub(crate) fn snapshot_processes(&self) -> Vec<ProcessSnapshot> {
        let mut rows: Vec<ProcessSnapshot> = Vec::new();
        self.tasks.visit(|task| {
            let state = task.state();
            if state == Terminated {
                return;
            }
            let group = task.group().value();
            if let Some(row) = rows.iter_mut().find(|row| row.group == group) {
                row.thread_count = row.thread_count.saturating_add(1);
                if liveness(state) > liveness(row.state) {
                    row.state = state;
                }
            } else {
                let name = task.name().as_bytes();
                let name_len = name.len().min(TASK_NAME_BYTES);
                let mut buffer = [0u8; TASK_NAME_BYTES];
                buffer[..name_len].copy_from_slice(&name[..name_len]);
                rows.push(ProcessSnapshot { group, state, thread_count: 1, name: buffer, name_len });
            }
        });
        rows.sort_by_key(|row| row.group);
        rows
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

    pub(crate) fn observe_task_stack(&mut self, handle: TaskHandle) {
        if let Ok(task) = self.tasks.borrow_mut(handle.into()) {
            match task.observe_stack() {
                StackObservation::WithinBudget => {}
                StackObservation::SpillIntoGuard => panic!(
                    "stack overflow: task '{}' suspended inside its stack guard (sp 0x{:x}, stack base 0x{:x}, {} words)",
                    task.name(),
                    task.stack_pointer(),
                    task.stack_base(),
                    STACK_WORDS
                ),
                StackObservation::Exhausted => panic!(
                    "stack overflow: task '{}' consumed all {} stack words (stack base 0x{:x})",
                    task.name(),
                    STACK_WORDS,
                    task.stack_base()
                ),
            }
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

    #[test]
    fn observe_task_stack_accepts_shallow_task() {
        let mut manager = TaskManager::new();
        let handle = manager.add_task(Task::new("shallow", 0, 0)).unwrap();
        manager.observe_task_stack(handle);
    }

    #[test]
    #[should_panic(expected = "stack overflow")]
    fn observe_task_stack_panics_when_saved_sp_sits_in_the_guard() {
        let mut manager = TaskManager::new();
        let handle = manager.add_task(Task::new("deep", 0, 0)).unwrap();
        let base = manager.borrow_task_mut(handle).unwrap().stack_base();
        manager.borrow_task_mut(handle).unwrap().set_stack_pointer(base + 8);
        manager.observe_task_stack(handle);
    }

    fn row_name(row: &ProcessSnapshot) -> &str {
        core::str::from_utf8(&row.name[..row.name_len]).unwrap()
    }

    #[test]
    fn snapshot_lists_one_row_per_group_with_its_name() {
        let mut manager = TaskManager::new();
        manager.add_task(Task::new("alpha", 0, 0)).unwrap();
        manager.add_task(Task::new("beta", 0, 0)).unwrap();
        let rows = manager.snapshot_processes();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().filter(|r| r.thread_count == 1).count(), 2);
        let mut names: [&str; 2] = [row_name(&rows[0]), row_name(&rows[1])];
        names.sort_unstable();
        assert_eq!(names, ["alpha", "beta"]);
    }

    #[test]
    fn snapshot_collapses_threads_into_one_process_row() {
        let mut manager = TaskManager::new();
        let leader = manager.add_task(Task::new("alpha", 0, 0)).unwrap();
        let group = manager.get_group(leader).unwrap();
        manager.add_task(new_entrypoint_task(0x1000, group)).unwrap();
        manager.add_task(new_entrypoint_task(0x2000, group)).unwrap();
        let rows = manager.snapshot_processes();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].thread_count, 3);
        assert_eq!(row_name(&rows[0]), "alpha");
    }

    #[test]
    fn snapshot_excludes_groups_with_no_live_members() {
        let mut manager = TaskManager::new();
        let handle = manager.add_task(Task::new("gone", 0, 0)).unwrap();
        manager.set_state(handle, Terminated);
        assert!(manager.snapshot_processes().is_empty());
    }

    #[test]
    fn snapshot_reports_the_most_active_thread_state() {
        let mut manager = TaskManager::new();
        let leader = manager.add_task(Task::new("mixed", 0, 0)).unwrap();
        let group = manager.get_group(leader).unwrap();
        let thread = manager.add_task(new_entrypoint_task(0x1000, group)).unwrap();
        manager.set_state(leader, TaskState::Blocked);
        manager.set_state(thread, TaskState::Running);
        let rows = manager.snapshot_processes();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, TaskState::Running);
    }
}
