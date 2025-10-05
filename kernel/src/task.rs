use alloc::boxed::Box;
use runnable::Runnable;
use task::TaskState::{Created, Ready, Running, Terminated};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskState {
    Created, Ready, Running, Blocked, Terminated,
}

pub type TaskEntryPoint = unsafe extern "C" fn() -> !;

pub struct Task {
    id: u32,
    name: &'static str,
    state: TaskState,
    stack_pointer: usize,
    entry_point: usize,
    stack: [u8; 1024],
}

impl Task {
    pub fn new(id: u32, name: &'static str, entry_point: usize) -> Task {
        unsafe {
            let mut stack: [u8; 1024] = [0; 1024];
            let stack_pointer = stack.as_mut_ptr().add(stack.len()).addr();
            Task {
                id,
                name,
                state: Created,
                stack_pointer,
                entry_point,
                stack,
            }
        }
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
    pub fn set_stack_pointer(&mut self, new_stack_pointer: usize) {
        self.stack_pointer = new_stack_pointer;
    }
    pub fn state(&self) -> TaskState {
        self.state
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
    pub fn is_schedulable(&self) -> bool {
        self.state() != Created && self.state() != Terminated
    }

    pub fn entry_point(&self) -> usize {
        self.entry_point
    }
}

impl PartialEq<Task> for Task {
    fn eq(&self, other: &Task) -> bool {
        self.id == other.id
    }
}

static mut NEXT_ID: u32 = 100;
pub fn next_id() -> u32 {
    unsafe  {
        NEXT_ID += 1;
        NEXT_ID
    }
}

