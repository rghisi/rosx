use task::SharedTask;

pub trait TaskScheduler {
    fn run(&mut self);

    fn push_task(&mut self, task: SharedTask);
}