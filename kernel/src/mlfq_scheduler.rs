use crate::task::{SharedTask, YieldReason};
use crate::task_queue::{EnqueuedTask, StateCreatedNotAccepted, TaskEnqueueingError, TaskQueue};
use alloc::collections::VecDeque;
use alloc::vec::Vec;

const NUM_QUEUES: usize = 3;

pub struct MlfqScheduler {
    queues: [VecDeque<SharedTask>; NUM_QUEUES],
}

impl MlfqScheduler {
    pub fn new() -> Self {
        MlfqScheduler {
            queues: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
        }
    }
}

impl TaskQueue for MlfqScheduler {
    fn offer(&mut self, mut task: SharedTask) -> Result<(), &dyn TaskEnqueueingError> {
        if !task.is_schedulable() {
            return Err(&StateCreatedNotAccepted);
        }
        let queue_index = match task.yield_reason() {
            None => 0,
            Some(YieldReason::Voluntary) => task.priority(),
            Some(YieldReason::Preempted) => (task.priority() + 1).min(NUM_QUEUES - 1),
        };
        task.set_priority(queue_index);
        self.queues[queue_index].push_back(task);
        Ok(())
    }

    fn take_next(&mut self) -> Option<SharedTask> {
        for queue in &mut self.queues {
            if let Some(task) = queue.pop_front() {
                return Some(task);
            }
        }
        None
    }

    fn list_tasks(&self) -> Vec<EnqueuedTask> {
        self.queues
            .iter()
            .flat_map(|q| q.iter().map(|task| EnqueuedTask { id: task.id(), name: task.name() }))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::MlfqScheduler;
    use crate::function_task::FunctionTask;
    use crate::task::YieldReason;
    use crate::task_queue::TaskQueue;

    fn dummy_job() {}

    #[test]
    fn new_task_is_placed_in_highest_priority_queue() {
        let mut scheduler = MlfqScheduler::new();

        let mut low_task = FunctionTask::new("Low", dummy_job);
        low_task.set_ready();
        low_task.set_yield_reason(YieldReason::Preempted);
        low_task.set_priority(1);
        let _ = scheduler.offer(low_task);

        let mut new_task = FunctionTask::new("New", dummy_job);
        new_task.set_ready();
        let _ = scheduler.offer(new_task);

        let next = scheduler.take_next().unwrap();
        assert_eq!(next.priority(), 0);
    }

    #[test]
    fn preempted_task_is_demoted_one_level() {
        let mut scheduler = MlfqScheduler::new();

        let mut task = FunctionTask::new("T", dummy_job);
        task.set_ready();
        let _ = scheduler.offer(task);

        let mut task = scheduler.take_next().unwrap();
        task.set_yield_reason(YieldReason::Preempted);
        let _ = scheduler.offer(task);

        let task = scheduler.take_next().unwrap();
        assert_eq!(task.priority(), 1);
    }

    #[test]
    fn preempted_task_at_lowest_priority_stays_at_lowest() {
        let mut scheduler = MlfqScheduler::new();

        let mut task = FunctionTask::new("T", dummy_job);
        task.set_ready();
        let _ = scheduler.offer(task);

        let mut task = scheduler.take_next().unwrap();
        task.set_yield_reason(YieldReason::Preempted);
        let _ = scheduler.offer(task);

        let mut task = scheduler.take_next().unwrap();
        task.set_yield_reason(YieldReason::Preempted);
        let _ = scheduler.offer(task);

        let mut task = scheduler.take_next().unwrap();
        task.set_yield_reason(YieldReason::Preempted);
        let _ = scheduler.offer(task);

        let task = scheduler.take_next().unwrap();
        assert_eq!(task.priority(), 2);
    }

    #[test]
    fn voluntary_yield_keeps_task_at_same_priority() {
        let mut scheduler = MlfqScheduler::new();

        let mut task = FunctionTask::new("T", dummy_job);
        task.set_ready();
        let _ = scheduler.offer(task);

        let mut task = scheduler.take_next().unwrap();
        assert_eq!(task.priority(), 0);
        task.set_yield_reason(YieldReason::Voluntary);
        let _ = scheduler.offer(task);

        let task = scheduler.take_next().unwrap();
        assert_eq!(task.priority(), 0);
    }

    #[test]
    fn higher_priority_task_runs_before_lower_priority() {
        let mut scheduler = MlfqScheduler::new();

        let mut task1 = FunctionTask::new("T1", dummy_job);
        task1.set_ready();
        let task1_id = task1.id();
        let _ = scheduler.offer(task1);

        let mut task1 = scheduler.take_next().unwrap();
        task1.set_yield_reason(YieldReason::Preempted);
        let _ = scheduler.offer(task1);

        let mut task2 = FunctionTask::new("T2", dummy_job);
        task2.set_ready();
        let task2_id = task2.id();
        let _ = scheduler.offer(task2);

        let next = scheduler.take_next().unwrap();
        assert_eq!(next.id(), task2_id);

        let next = scheduler.take_next().unwrap();
        assert_eq!(next.id(), task1_id);
    }

    #[test]
    fn task_with_cleared_yield_reason_is_boosted_to_highest_priority() {
        let mut scheduler = MlfqScheduler::new();

        let mut task = FunctionTask::new("T", dummy_job);
        task.set_ready();
        let _ = scheduler.offer(task);

        let mut task = scheduler.take_next().unwrap();
        task.set_yield_reason(YieldReason::Preempted);
        let _ = scheduler.offer(task);

        let mut task = scheduler.take_next().unwrap();
        assert_eq!(task.priority(), 1);
        task.clear_yield_reason();
        let _ = scheduler.offer(task);

        let task = scheduler.take_next().unwrap();
        assert_eq!(task.priority(), 0);
    }

    #[test]
    fn returns_none_when_all_queues_are_empty() {
        let mut scheduler = MlfqScheduler::new();
        assert!(scheduler.take_next().is_none());
    }

    #[test]
    fn created_or_terminated_task_is_rejected() {
        let mut scheduler = MlfqScheduler::new();

        let created_task = FunctionTask::new("Created", dummy_job);
        assert!(scheduler.offer(created_task).is_err());

        let mut terminated_task = FunctionTask::new("Terminated", dummy_job);
        terminated_task.set_terminated();
        assert!(scheduler.offer(terminated_task).is_err());
    }
}
