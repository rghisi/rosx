use collections::generational_arena::Handle;
use crate::task::{TaskHandle, YieldReason};
use super::algorithm::SchedulingAlgorithm;

pub struct FifoStrategy {
    ready_queue: alloc::collections::VecDeque<TaskHandle>,
}

impl Default for FifoStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl FifoStrategy {
    pub fn new() -> Self {
        FifoStrategy {
            ready_queue: alloc::collections::VecDeque::new(),
        }
    }
}

impl SchedulingAlgorithm for FifoStrategy {
    fn pick_next(&mut self) -> Option<TaskHandle> {
        self.ready_queue.pop_front()
    }

    fn record_yield(&mut self, _handle: TaskHandle, _yield_reason: YieldReason) {
    }

    fn requeue_after_run(&mut self, handle: TaskHandle) {
        self.ready_queue.push_back(handle);
    }

    fn push_ready(&mut self, handle: TaskHandle) {
        self.ready_queue.push_back(handle);
    }

    fn should_preempt(&mut self) -> bool {
        true
    }

    fn on_task_start(&mut self, _handle: TaskHandle) {
    }

    fn on_task_terminate(&mut self, handle: TaskHandle) {
        if let Some(pos) = self.ready_queue.iter().position(|&h| h == handle) {
            self.ready_queue.remove(pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handle(index: u32, generation: u32) -> TaskHandle {
        Handle::new(index, generation)
    }

    #[test]
    fn new_fifo_strategy_is_empty() {
        let mut strategy = FifoStrategy::new();
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn default_fifo_strategy_is_empty() {
        let mut strategy = FifoStrategy::default();
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn push_ready_adds_task_to_back_of_queue() {
        let mut strategy = FifoStrategy::new();
        let h1 = make_handle(1, 0);
        let h2 = make_handle(2, 0);

        strategy.push_ready(h1);
        strategy.push_ready(h2);

        assert_eq!(strategy.pick_next(), Some(h1));
        assert_eq!(strategy.pick_next(), Some(h2));
    }

    #[test]
    fn pick_next_returns_tasks_in_fifo_order() {
        let mut strategy = FifoStrategy::new();
        let h3 = make_handle(3, 0);
        let h1 = make_handle(1, 0);
        let h2 = make_handle(2, 0);

        strategy.push_ready(h3);
        strategy.push_ready(h1);
        strategy.push_ready(h2);

        assert_eq!(strategy.pick_next(), Some(h3));
        assert_eq!(strategy.pick_next(), Some(h1));
        assert_eq!(strategy.pick_next(), Some(h2));
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn requeue_after_run_adds_task_to_back() {
        let mut strategy = FifoStrategy::new();
        let h = make_handle(42, 0);

        strategy.push_ready(h);
        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h);

        strategy.requeue_after_run(h);
        assert_eq!(strategy.pick_next(), Some(h));
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn should_preempt_always_returns_true() {
        let mut strategy = FifoStrategy::new();
        assert!(strategy.should_preempt());
        assert!(strategy.should_preempt());
        assert!(strategy.should_preempt());
    }

    #[test]
    fn on_task_start_is_noop() {
        let mut strategy = FifoStrategy::new();
        strategy.on_task_start(make_handle(1, 0));
        strategy.on_task_start(make_handle(2, 0));
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn record_yield_is_noop() {
        let mut strategy = FifoStrategy::new();
        let h = make_handle(1, 0);
        strategy.record_yield(h, YieldReason::Voluntary);
        strategy.record_yield(h, YieldReason::Preempted);
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn on_task_terminate_removes_handle_from_queue() {
        let mut strategy = FifoStrategy::new();
        let h1 = make_handle(1, 0);
        let h2 = make_handle(2, 0);

        strategy.push_ready(h1);
        strategy.push_ready(h2);
        assert_eq!(strategy.ready_queue.len(), 2);

        strategy.on_task_terminate(h1);
        assert_eq!(strategy.ready_queue.len(), 1);
        assert_eq!(strategy.pick_next(), Some(h2));
    }

    #[test]
    fn on_task_terminate_on_nonexistent_handle_is_noop() {
        let mut strategy = FifoStrategy::new();
        let h = make_handle(99, 0);
        strategy.on_task_terminate(h); // should not panic
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn push_ready_and_requeue_after_run_interleave_correctly() {
        let mut strategy = FifoStrategy::new();
        let h1 = make_handle(1, 0);
        let h2 = make_handle(2, 0);
        let h3 = make_handle(3, 0);

        strategy.push_ready(h1);
        strategy.push_ready(h2);

        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h1);
        strategy.requeue_after_run(h1);

        strategy.push_ready(h3);

        assert_eq!(strategy.pick_next(), Some(h2));
        assert_eq!(strategy.pick_next(), Some(h1));
        assert_eq!(strategy.pick_next(), Some(h3));
    }
}
