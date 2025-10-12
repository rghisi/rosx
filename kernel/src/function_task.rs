use alloc::boxed::Box;
use task::{Task, next_id};
use wrappers::task_wrapper;

#[derive(Copy, Clone, Debug)]
pub struct FunctionTask {
}

impl FunctionTask {
    pub fn new(name: &'static str, job: fn()) -> Box<Task> {
        Task::new(
            next_id(),
            name,
            task_wrapper as usize,
            job as usize
        )
    }
}
