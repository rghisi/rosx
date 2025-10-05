use task::{Task, next_id};

#[derive(Copy, Clone, Debug)]
pub struct FunctionTask {
    job: fn(),
}

impl FunctionTask {
    pub fn new(name: &'static str, job: fn()) -> Task {
        Task::new(
            next_id(),
            name,
            job as usize
        )
    }
}
