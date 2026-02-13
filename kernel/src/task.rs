use collections::generational_arena::Handle;
use crate::kernel::task_wrapper;
use crate::task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use alloc::boxed::Box;
use core::fmt::{Display, Formatter};

pub(crate) type TaskHandle = Handle<u8, u8>;
pub type SharedTask = Box<Task>;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum TaskState {
    Created,
    Ready,
    Running,
    Blocked,
    Terminated,
}

impl Display for TaskState {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Created => {
                write!(f, "Created")
            }
            Ready => {
                write!(f, "Ready")
            }
            Running => {
                write!(f, "Running")
            }
            TaskState::Blocked => {
                write!(f, "Blocked")
            }
            Terminated => {
                write!(f, "Terminated")
            }
        }
    }
}
pub struct Task {
    id: u32,
    name: &'static str,
    state: TaskState,
    stack_pointer: usize,
    entry_point_wrapper: usize,
    actual_entry_point: usize,
    stack: [usize; 2048], //16KB on 64bit systems
}

impl Task {
    pub fn new<'a>(
        id: u32,
        name: &'static str,
        entry_wrapper: usize,
        entry_point: usize,
    ) -> SharedTask {
        let mut task = Box::new(Task {
            id,
            name,
            state: Created,
            stack_pointer: 0,
            entry_point_wrapper: entry_wrapper,
            actual_entry_point: entry_point,
            stack: [0; 2048],
        });

        unsafe {
            let stack_pointer = task.stack.as_mut_ptr().add(task.stack.len()).addr();
            task.set_stack_pointer(stack_pointer);
        }

        task
    }
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn name(&self) -> &'static str {
        self.name
    }
    pub fn stack_pointer(&self) -> usize {
        self.stack_pointer
    }
    pub fn stack_pointer_mut(&mut self) -> *mut usize {
        &mut self.stack_pointer as *mut usize
    }
    pub fn set_stack_pointer(&mut self, new_stack_pointer: usize) {
        self.stack_pointer = new_stack_pointer;
    }
    pub fn state(&self) -> TaskState {
        self.state
    }

    pub fn set_state(&mut self, new_state: TaskState) {
        self.state = new_state;
    }
    pub fn set_ready(&mut self) {
        self.state = Ready;
    }
    pub fn set_running(&mut self) {
        self.state = Running;
    }
    pub fn set_terminated(&mut self) {
        self.state = Terminated;
    }

    pub fn set_blocked(&mut self) {
        self.state = Blocked;
    }
    pub fn is_schedulable(&self) -> bool {
        self.state() != Created && self.state() != Terminated
    }

    pub fn entry_point(&self) -> usize {
        self.entry_point_wrapper
    }
    pub fn actual_entry_point(&self) -> usize {
        self.actual_entry_point
    }
}

pub struct EntrypointTask {
    entry_point: usize,
}

impl EntrypointTask {
    pub fn new(entrypoint: usize) -> SharedTask {
        Task::new(next_id(), "EPT", task_wrapper as usize, entrypoint)
    }
}

static mut NEXT_ID: u32 = 100;
pub fn next_id() -> u32 {
    unsafe {
        NEXT_ID += 1;
        NEXT_ID
    }
}
