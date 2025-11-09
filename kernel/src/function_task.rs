use crate::kernel::task_wrapper;
use crate::task::{Task, next_id, SharedTask};

#[derive(Copy, Clone, Debug)]
pub struct FunctionTask {
}

impl FunctionTask {
    pub fn new(name: &'static str, job: fn()) -> SharedTask {
        Task::new(
            next_id(),
            name,
            task_wrapper as usize,
            job as usize
        )
    }
}
