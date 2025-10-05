use alloc::boxed::Box;
use alloc::vec::Vec;
use runnable::Runnable;
use scheduler::Scheduler;
use simple_scheduler::SimpleScheduler;
use task::{Task, TaskEntryPoint};
use task::TaskState::{Blocked, Created, Ready, Running};

pub(crate) struct MainThread {
    idle_task: Box<Task>,
    ready_tasks: SimpleScheduler,
    blocked_tasks: Vec<Box<Task>>,
}

impl MainThread {

    pub(crate) fn new(idle_task: Box<Task>) -> Self {
        MainThread {
            idle_task,
            ready_tasks: SimpleScheduler::new(),
            blocked_tasks: Vec::with_capacity(5),
        }
    }

    pub(crate) fn push_task(&mut self, task: Box<Task>) {
        let _ = match task.state() {
            Ready => self.ready_tasks.offer(task),
            // Blocked => self.blocked_tasks.push(task),
            _ => return
        };
    }

    pub fn get_vtable_entry_address(obj: &MainThread) -> usize {
        let trait_object: &dyn Runnable = obj;

        // 1. Convert the trait object (fat pointer) to its raw components
        let raw_fat_ptr: (*const MainThread, *const ()) = unsafe {
            core::mem::transmute(trait_object)
        };

        let vtable_ptr = raw_fat_ptr.1;

        // 2. Read the function address from the vtable.
        // The vtable layout is: [metadata_ptr, run_fn_ptr, other_fns_ptr...]
        // The first entry (offset 0) is usually the Run function pointer.
        unsafe {
            let run_fn_ptr_ptr = vtable_ptr as *const *const ();
            // Read the actual function address
            let run_fn_address = *run_fn_ptr_ptr;

            run_fn_address as usize
        }
    }
}

impl Runnable for MainThread {
    fn run(&mut self) {
        loop {

        }
    }
}
