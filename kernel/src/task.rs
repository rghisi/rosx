use alloc::boxed::Box;
use core::fmt::{Display, Formatter};
use kernel::CURRENT_TASK;
use kprintln;
use runnable::Runnable;
use task::TaskState::{Created, Ready, Running, Terminated};

pub type SharedTask = Box<Task>;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskState {
    Created, Ready, Running, Blocked, Terminated,
}

impl Display for TaskState {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Created => {write!(f, "Created")}
            Ready => {write!(f, "Ready")}
            Running => {write!(f, "Running")}
            TaskState::Blocked => {write!(f, "Blocked")}
            Terminated => {write!(f, "Terminated")}
        }
    }
}

pub type TaskEntryPoint = unsafe extern "C" fn() -> !;

// Wrapper function that calls the actual task entry and handles completion
// Takes the actual entry point as a parameter (passed via RDI in x86_64)
// Calls task_yield to return control to MainThread when done
extern "C" fn task_wrapper(actual_entry: usize) {
    let task_fn: fn() = unsafe { core::mem::transmute(actual_entry) };
    task_fn();

    unsafe {
        if let Some(mut task) = CURRENT_TASK.take() {
            task.set_terminated();
            CURRENT_TASK = Some(task);
        }
    }

    crate::kernel::task_yield();
}

#[inline(always)]
pub fn with_current_task<F>(f: F)
where
    F: FnOnce(&mut Task),
{
    unsafe {
        if let Some(mut task) = CURRENT_TASK.take() {
            f(&mut task);
            CURRENT_TASK = Some(task);
        }
    }
}

pub struct Task {
    id: u32,
    name: &'static str,
    state: TaskState,
    stack_pointer: usize,
    entry_point: usize,
    actual_entry_point: usize,
    stack: [usize; 256],
}

impl Task {
    pub fn new(id: u32, name: &'static str, actual_entry_point: usize) -> SharedTask {
        let mut task = Box::new(Task {
            id,
            name,
            state: Created,
            stack_pointer: 0,
            entry_point: task_wrapper as usize,
            actual_entry_point,
            stack: [0; 256],
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
    pub fn actual_entry_point(&self) -> usize {
        self.actual_entry_point
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

impl Drop for Task {
    fn drop(&mut self) {
    }
}