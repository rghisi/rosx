use alloc::boxed::Box;
use core::fmt::{Display, Formatter};
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
    kprintln!("[TASK_WRAPPER] Starting task");

    // Call the actual task function
    let task_fn: fn() = unsafe { core::mem::transmute(actual_entry) };
    task_fn();

    kprintln!("[TASK_WRAPPER] Task completed, marking as terminated");

    // Mark task as terminated before yielding
    unsafe {
        if let Some(current_task_ptr) = crate::kernel::CURRENT_TASK_PTR {
            (*current_task_ptr).set_terminated();
        }
    }

    // Yield back to MainThread
    crate::kernel::task_yield();
}

pub struct Task {
    id: u32,
    name: &'static str,
    state: TaskState,
    stack_pointer: usize,
    entry_point: usize,        // The wrapper function address
    actual_entry_point: usize, // The actual task function to be called by wrapper
    stack: [u8; 1024],
}

impl Task {
    pub fn new(id: u32, name: &'static str, actual_entry_point: usize) -> SharedTask {
        let mut task = Box::new(Task {
            id,
            name,
            state: Created,
            stack_pointer: 0,  // Will be set correctly below
            entry_point: task_wrapper as usize,  // Use wrapper as the entry point
            actual_entry_point,                   // Store the actual task function
            stack: [0; 1024],
        });

        // Now that the task is in its final location on the heap,
        // calculate the stack pointer based on the actual stack buffer address
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

