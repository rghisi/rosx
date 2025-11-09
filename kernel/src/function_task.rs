use crate::kernel::task_wrapper;
use crate::task::{SharedTask, Task, next_id};

#[derive(Copy, Clone, Debug)]
pub struct FunctionTask {}

impl FunctionTask {
    pub fn new(name: &'static str, job: fn()) -> SharedTask {
        Task::new(next_id(), name, task_wrapper as usize, job as usize)
    }
}
