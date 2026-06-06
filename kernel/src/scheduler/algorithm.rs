use crate::task::{TaskHandle, YieldReason};

pub trait SchedulingAlgorithm {
    /// Pick the next task to run. Returns None if no user tasks are ready.
    fn pick_next(&mut self) -> Option<TaskHandle>;

    /// Record how a task yielded so the strategy can decide requeue placement.
    fn record_yield(&mut self, handle: TaskHandle, current_priority: usize, yield_reason: YieldReason);

    /// Requeue a task after it returns from running.
    /// Uses info previously recorded via `record_yield`.
    fn requeue_after_run(&mut self, handle: TaskHandle);

    /// Push a newly-ready task into this strategy's ready queue(s).
    fn push_ready(&mut self, handle: TaskHandle);

    /// Called during timer ticks to decide if the current task should be preempted.
    fn should_preempt(&mut self) -> bool;

    /// Called when a task is about to start running (e.g., reset quantum in MLFQ).
    fn on_task_start(&mut self, priority: usize);
}
