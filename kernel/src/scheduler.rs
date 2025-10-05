use alloc::boxed::Box;
use alloc::vec::Vec;
use task::Task;

pub trait Scheduler {
    fn offer(&mut self, task: Box<Task>) -> Result<(), &dyn SchedulingError>;
    fn take_next(&mut self) -> Option<Box<Task>>;

    fn list_tasks(&self) -> Vec<ScheduledTask>;
}

pub trait SchedulingError {

}

pub struct StateCreatedNotAccepted;

impl SchedulingError for StateCreatedNotAccepted {

}

pub struct ScheduledTask {
    pub id: u32,
    pub name: &'static str,
}



