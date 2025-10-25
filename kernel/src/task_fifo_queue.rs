use alloc::vec::Vec;
use task_queue::{EnqueuedTask, TaskQueue, TaskEnqueueingError, StateCreatedNotAccepted};
use task::{SharedTask, Task};

pub struct TaskFifoQueue {
    tasks: Vec<SharedTask>,
}

impl TaskFifoQueue {
    pub fn new() -> Self {
        TaskFifoQueue {
            tasks: Vec::with_capacity(5),
        }
    }
}

impl TaskQueue for TaskFifoQueue {
    fn offer(&mut self, task: SharedTask) -> Result<(), &dyn TaskEnqueueingError> {
        if !task.is_schedulable() {
            return Err(&StateCreatedNotAccepted);
        }

        self.tasks.push(task);

        Ok(())
    }

    fn take_next(&mut self) -> Option<SharedTask> {
        if self.tasks.is_empty() {
            None
        } else {
            Some(self.tasks.remove(0))
        }
    }
    fn list_tasks(&self) -> Vec<EnqueuedTask> {
        self.tasks
            .as_slice()
            .iter()
            .map(|task| EnqueuedTask {
                id: task.id(),
                name: task.name(),
            })
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use alloc::boxed::Box;
    use std::fmt::{Debug, Formatter};
    use std::ptr::eq;
    use super::TaskFifoQueue;
    use task_queue::TaskQueue;
    use task::Task;
    use crate::function_task::FunctionTask;
    use task::TaskState::{Created, Ready, Running, Terminated};

    fn dummy_job() {}

    #[test]
    fn should_return_none_when_no_tasks_are_available() {
        let mut scheduler: TaskFifoQueue = TaskFifoQueue::new();

        let next_task = scheduler.take_next();

        assert_eq!(None, next_task);
    }

    #[test]
    fn should_return_task_when_its_the_only_one_ever_offered() {
        let mut task = FunctionTask::new("Any task", dummy_job);
        let task_id = task.id();
        task.set_ready();
        let mut scheduler = TaskFifoQueue::new();

        let result = scheduler.offer(task);
        assert!(result.is_ok());

        let next_task = scheduler.take_next().unwrap();
        assert_eq!(next_task.id(), task_id);
    }

    #[test]
    fn should_round_robin_tasks_when_more_than_one_available() {
        let mut scheduler = TaskFifoQueue::new();
        let mut task1_id: u32 = u32::MAX;
        let mut task2_id: u32 = u32::MAX;
        {
            let mut task1 = FunctionTask::new("T1", dummy_job);
            task1_id = task1.id();
            task1.set_ready();
            let mut task2 = FunctionTask::new("T2", dummy_job);
            task2_id = task2.id();
            task2.set_ready();

            let _ = scheduler.offer(task1);
            let _ = scheduler.offer(task2);
        }

        {
            let t1a = scheduler.take_next().unwrap();
            assert_eq!(t1a.id(), task1_id);
            let _ = scheduler.offer(t1a);
        }
        {
            let t2a = scheduler.take_next().unwrap();
            assert_eq!(t2a.id(), task2_id);
            let _ = scheduler.offer(t2a);
        }

        {
            let t1b = scheduler.take_next().unwrap();
            assert_eq!(t1b.id(), task1_id);
            let _ = scheduler.offer(t1b);
        }
        {
            let t2b = scheduler.take_next().unwrap();
            assert_eq!(t2b.id(), task2_id);
        }
    }

    #[test]
    fn should_preserve_task_state_when_task_is_changed() {
        let mut scheduler = TaskFifoQueue::new();
        {
            let mut task1 = FunctionTask::new("T1", dummy_job);
            task1.set_ready();
            let _ = scheduler.offer(task1);
        }

        {
            let mut task1 = scheduler.take_next().unwrap();
            assert_eq!(task1.state(), Ready);
            task1.set_running();
            let t1_id = task1.id();
            let _ = scheduler.offer(task1);
            let task1_b = scheduler.take_next().unwrap();
            assert_eq!(task1_b.id(), t1_id);
            assert_eq!(task1_b.state(), Running);
        }
    }

    #[test]
    fn should_result_scheduling_error_when_task_state_is_created() {
        let mut scheduler = TaskFifoQueue::new();
        let task1 = FunctionTask::new("T1", dummy_job);
        let task_state = task1.state();
        let result = scheduler.offer(task1);
        assert_eq!(task_state, Created);
        assert_eq!(result.is_err(), true);
    }

    #[test]
    fn should_result_scheduling_error_when_task_state_is_terminated() {
        let mut scheduler = TaskFifoQueue::new();
        let mut task1 = FunctionTask::new("T1", dummy_job);
        task1.set_terminated();
        let task_state = task1.state();
        let result = scheduler.offer(task1);
        assert_eq!(task_state, Terminated);
        assert_eq!(result.is_err(), true);
    }

    impl Debug for Task {
        fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
            f.write_str(self.name())
        }
    }

    impl PartialEq for Task {
        fn eq(&self, other: &Self) -> bool {
            self.id() == other.id()
        }
    }
    
}
