use alloc::boxed::Box;
use alloc::vec::Vec;
use task::Task;

pub trait TaskQueue {
    fn offer(&mut self, task: Box<Task>) -> Result<(), &dyn TaskEnqueueingError>;
    fn take_next(&mut self) -> Option<Box<Task>>;

    fn list_tasks(&self) -> Vec<EnqueuedTask>;
}

pub trait TaskEnqueueingError {

}

pub struct StateCreatedNotAccepted;

impl TaskEnqueueingError for StateCreatedNotAccepted {

}

pub struct EnqueuedTask {
    pub id: u32,
    pub name: &'static str,
}



