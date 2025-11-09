use core::ptr::null_mut;
use crate::generational_arena::GenArena;
use crate::task::{EntrypointTask, SharedTask, Task, TaskHandle, TaskState};
use crate::task::TaskState::Terminated;

pub(crate) struct TaskManager {
    tasks: GenArena<SharedTask, u8, u8>
}

#[derive(Debug)]
pub(crate) enum Error {
    TaskCannotBeAdded,
    TaskNotFound,
}

impl TaskManager {
    pub(crate) fn new() -> Self {
        TaskManager {
            tasks: GenArena::new(10)
        }
    }

    pub(crate) fn new_task(&mut self, entrypoint: usize) -> Result<TaskHandle, Error> {
        let new_task = EntrypointTask::new(entrypoint);
        self.add_task(new_task)
    }

    pub(crate) fn add_task(&mut self, task: SharedTask) -> Result<TaskHandle, Error> {
        match self.tasks.add(task) {
            Ok(handle) => Ok(handle),
            Err(_) => Err(Error::TaskCannotBeAdded)
        }
    }

    pub(crate) fn remove_task(&mut self, handle: TaskHandle) {
        let _ = self.tasks.remove(handle);
    }

    pub(crate) fn borrow_task(&mut self, handle: TaskHandle) -> Result<&Task, Error> {
        match self.tasks.borrow(handle) {
            Ok(task) => Ok(task),
            Err(_) => Err(Error::TaskNotFound)
        }
    }

    pub(crate) fn borrow_task_mut(&mut self, handle: TaskHandle) -> Result<&mut Task, Error> {
        match self.tasks.borrow_mut(handle) {
            Ok(task) => Ok(task),
            Err(_) => Err(Error::TaskNotFound)
        }
    }

    pub(crate) fn get_state(&self, handle: TaskHandle) -> TaskState {
        match self.tasks.borrow(handle) {
            Ok(task) => task.state(),
            Err(_) => Terminated
        }
    }

    pub(crate) fn set_state(&mut self, handle: TaskHandle, state: TaskState) {
        match self.tasks.borrow_mut(handle) {
            Ok(task) => task.set_state(state),
            Err(_) => {}
        }
    }

    pub(crate) fn get_task_stack_pointer(&self, handle: TaskHandle) -> usize {
        match self.tasks.borrow(handle) {
            Ok(task) => task.stack_pointer(),
            Err(_) => 0
        }
    }

    pub(crate) fn get_task_stack_pointer_ref(&mut self, handle: TaskHandle) -> *mut usize {
        match self.tasks.borrow_mut(handle) {
            Ok(task) => task.stack_pointer_mut(),
            Err(_) => { null_mut() }
        }
    }
}
