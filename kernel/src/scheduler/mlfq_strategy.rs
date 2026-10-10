use alloc::collections::{BTreeMap, VecDeque};
use collections::generational_arena::Handle;
use crate::task::{TaskHandle, YieldReason};
use super::algorithm::SchedulingAlgorithm;

const NUM_QUEUES: usize = 3;
const QUANTA: [usize; NUM_QUEUES] = [2, 5, 10];

#[derive(Clone, Copy)]
struct YieldInfo {
    yield_reason: YieldReason,
    priority: usize,
}

pub struct MlfqStrategy {
    queues: [VecDeque<TaskHandle>; NUM_QUEUES],
    remaining_quantum: usize,
    current_priority: usize,
    yield_info: BTreeMap<TaskHandle, YieldInfo>,
}

impl Default for MlfqStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl MlfqStrategy {
    pub fn new() -> Self {
        MlfqStrategy {
            queues: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
            remaining_quantum: 0,
            current_priority: 0,
            yield_info: BTreeMap::new(),
        }
    }

    fn next_priority(current: usize, yield_reason: Option<YieldReason>) -> usize {
        match yield_reason {
            None => 0,
            Some(YieldReason::Voluntary) => current,
            Some(YieldReason::Preempted) => (current + 1).min(NUM_QUEUES - 1),
        }
    }
}

impl SchedulingAlgorithm for MlfqStrategy {
    fn pick_next(&mut self) -> Option<TaskHandle> {
        for (priority, queue) in self.queues.iter_mut().enumerate() {
            if let Some(handle) = queue.pop_front() {
                self.current_priority = priority;
                return Some(handle);
            }
        }
        None
    }

    fn record_yield(&mut self, handle: TaskHandle, yield_reason: YieldReason) {
        self.yield_info.insert(handle, YieldInfo {
            yield_reason,
            priority: self.current_priority,
        });
    }

    fn requeue_after_run(&mut self, handle: TaskHandle) {
        if let Some(info) = self.yield_info.remove(&handle) {
            let new_priority = Self::next_priority(info.priority, Some(info.yield_reason));
            self.queues[new_priority].push_back(handle);
        } else {
            self.queues[0].push_back(handle);
        }
    }

    fn push_ready(&mut self, handle: TaskHandle) {
        self.queues[0].push_back(handle);
    }

    fn should_preempt(&mut self) -> bool {
        self.remaining_quantum = self.remaining_quantum.saturating_sub(1);
        self.remaining_quantum == 0
    }

    fn on_task_start(&mut self, _handle: TaskHandle) {
        self.remaining_quantum = QUANTA[self.current_priority];
    }

    fn on_task_terminate(&mut self, handle: TaskHandle) {
        for queue in &mut self.queues {
            if let Some(pos) = queue.iter().position(|&h| h == handle) {
                queue.remove(pos);
            }
        }
        self.yield_info.remove(&handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handle(index: u16, generation: u16) -> TaskHandle {
        Handle::new(index, generation)
    }

    // === next_priority tests ===

    #[test]
    fn next_priority_with_no_yield_reason_returns_queue_0() {
        assert_eq!(MlfqStrategy::next_priority(1, None), 0);
        assert_eq!(MlfqStrategy::next_priority(2, None), 0);
    }

    #[test]
    fn next_priority_with_voluntary_keeps_current_priority() {
        assert_eq!(MlfqStrategy::next_priority(0, Some(YieldReason::Voluntary)), 0);
        assert_eq!(MlfqStrategy::next_priority(1, Some(YieldReason::Voluntary)), 1);
        assert_eq!(MlfqStrategy::next_priority(2, Some(YieldReason::Voluntary)), 2);
    }

    #[test]
    fn next_priority_with_preempted_demotes_one_level() {
        assert_eq!(MlfqStrategy::next_priority(0, Some(YieldReason::Preempted)), 1);
        assert_eq!(MlfqStrategy::next_priority(1, Some(YieldReason::Preempted)), 2);
    }

    #[test]
    fn next_priority_with_preempted_at_lowest_stays_at_lowest() {
        assert_eq!(MlfqStrategy::next_priority(2, Some(YieldReason::Preempted)), 2);
    }

    // === new/empty state tests ===

    #[test]
    fn new_mlfq_strategy_has_empty_queues() {
        let mut strategy = MlfqStrategy::new();
        for i in 0..NUM_QUEUES {
            assert_eq!(strategy.queues[i].len(), 0);
        }
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn default_mlfq_strategy_has_empty_queues() {
        let mut strategy = MlfqStrategy::default();
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn pick_next_tracks_current_priority() {
        let mut strategy = MlfqStrategy::new();
        let h0 = make_handle(1, 0);
        let h1 = make_handle(2, 0);
        let h2 = make_handle(3, 0);

        strategy.queues[0].push_back(h0);
        strategy.queues[1].push_back(h1);
        strategy.queues[2].push_back(h2);

        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h0);
        assert_eq!(strategy.current_priority, 0);

        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h1);
        assert_eq!(strategy.current_priority, 1);

        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h2);
        assert_eq!(strategy.current_priority, 2);
    }

    // === push_ready tests ===

    #[test]
    fn push_ready_adds_task_to_queue_0() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);
        strategy.push_ready(h);
        assert_eq!(strategy.queues[0].len(), 1);
        for i in 1..NUM_QUEUES {
            assert_eq!(strategy.queues[i].len(), 0);
        }
    }

    #[test]
    fn push_ready_adds_multiple_tasks_to_queue_0_in_order() {
        let mut strategy = MlfqStrategy::new();
        let h1 = make_handle(1, 0);
        let h2 = make_handle(2, 0);
        let h3 = make_handle(3, 0);

        strategy.push_ready(h1);
        strategy.push_ready(h2);
        strategy.push_ready(h3);

        assert_eq!(strategy.queues[0].len(), 3);
        assert_eq!(strategy.queues[0][0], h1);
        assert_eq!(strategy.queues[0][1], h2);
        assert_eq!(strategy.queues[0][2], h3);
    }

    // === pick_next tests ===

    #[test]
    fn pick_next_returns_from_highest_priority_non_empty_queue() {
        let mut strategy = MlfqStrategy::new();
        let h_queue0 = make_handle(1, 0);
        let h_queue1 = make_handle(2, 0);

        strategy.push_ready(h_queue0);
        strategy.queues[1].push_back(h_queue1);

        // Queue 0 is highest priority, so h_queue0 is picked first
        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h_queue0);
        assert_eq!(strategy.current_priority, 0);
    }

    #[test]
    fn pick_next_iterates_queues_in_priority_order() {
        let mut strategy = MlfqStrategy::new();
        let h0 = make_handle(1, 0);
        let h1 = make_handle(2, 0);
        let h2 = make_handle(3, 0);

        strategy.queues[0].push_back(h0);
        strategy.queues[1].push_back(h1);
        strategy.queues[2].push_back(h2);

        // Queue 0 is highest priority
        assert_eq!(strategy.pick_next(), Some(h0));
        assert_eq!(strategy.current_priority, 0);
        assert_eq!(strategy.pick_next(), Some(h1));
        assert_eq!(strategy.current_priority, 1);
        assert_eq!(strategy.pick_next(), Some(h2));
        assert_eq!(strategy.current_priority, 2);
        assert!(strategy.pick_next().is_none());
    }

    #[test]
    fn pick_next_returns_none_when_all_queues_empty() {
        let mut strategy = MlfqStrategy::new();
        assert!(strategy.pick_next().is_none());
    }

    // === record_yield tests ===

    #[test]
    fn record_yield_stores_yield_reason_and_current_priority() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);

        strategy.current_priority = 1;
        strategy.record_yield(h, YieldReason::Voluntary);

        let info = strategy.yield_info.get(&h).unwrap();
        assert_eq!(info.yield_reason, YieldReason::Voluntary);
        assert_eq!(info.priority, 1);
    }

    #[test]
    fn record_yield_overwrites_previous_info_for_same_handle() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);

        strategy.current_priority = 0;
        strategy.record_yield(h, YieldReason::Voluntary);
        strategy.current_priority = 2;
        strategy.record_yield(h, YieldReason::Preempted);

        let info = strategy.yield_info.get(&h).unwrap();
        assert_eq!(info.yield_reason, YieldReason::Preempted);
        assert_eq!(info.priority, 2);
    }

    // === requeue_after_run tests ===

    #[test]
    fn requeue_after_voluntary_yield_keeps_same_priority() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);

        strategy.push_ready(h);
        let picked = strategy.pick_next().unwrap();
        strategy.record_yield(picked, YieldReason::Voluntary);
        strategy.requeue_after_run(picked);

        assert_eq!(strategy.queues[0].len(), 1);
        assert_eq!(strategy.queues[1].len(), 0);
        assert_eq!(strategy.queues[2].len(), 0);
    }

    #[test]
    fn requeue_after_preemption_demotes_to_next_level() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);

        strategy.push_ready(h);
        let picked = strategy.pick_next().unwrap();
        strategy.record_yield(picked, YieldReason::Preempted);
        strategy.requeue_after_run(picked);

        assert_eq!(strategy.queues[0].len(), 0);
        assert_eq!(strategy.queues[1].len(), 1);
        assert_eq!(strategy.queues[2].len(), 0);
    }

    #[test]
    fn requeue_at_lowest_priority_stays_at_lowest_when_preempted() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);

        // Push directly to queue 2 (lowest priority)
        strategy.queues[2].push_back(h);
        let picked = strategy.pick_next().unwrap();
        assert_eq!(strategy.current_priority, 2);
        strategy.record_yield(picked, YieldReason::Preempted);
        strategy.requeue_after_run(picked);

        // Should stay at lowest priority (queue 2)
        assert_eq!(strategy.queues[2].len(), 1);
        assert_eq!(strategy.queues[0].len(), 0);
        assert_eq!(strategy.queues[1].len(), 0);
    }

    #[test]
    fn requeue_after_run_removes_yield_info() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);

        strategy.push_ready(h);
        let picked = strategy.pick_next().unwrap();
        strategy.record_yield(picked, YieldReason::Preempted);
        strategy.requeue_after_run(picked);

        assert!(!strategy.yield_info.contains_key(&h));
    }

    // === should_preempt tests ===

    #[test]
    fn should_preempt_returns_false_while_quantum_remaining() {
        let mut strategy = MlfqStrategy::new();
        strategy.remaining_quantum = QUANTA[0];

        for _ in 0..QUANTA[0] - 1 {
            assert!(!strategy.should_preempt());
        }
    }

    #[test]
    fn should_preempt_returns_true_when_quantum_exhausted() {
        let mut strategy = MlfqStrategy::new();
        strategy.remaining_quantum = QUANTA[0];

        for _ in 0..QUANTA[0] - 1 {
            strategy.should_preempt();
        }

        assert!(strategy.should_preempt());
    }

    #[test]
    fn should_preempt_saturates_at_zero_after_exhaustion() {
        let mut strategy = MlfqStrategy::new();
        strategy.remaining_quantum = QUANTA[0];

        for _ in 0..QUANTA[0] {
            strategy.should_preempt();
        }

        assert!(strategy.should_preempt());
        assert!(strategy.should_preempt());
    }

    #[test]
    fn should_preempt_respects_quantum_for_each_priority() {
        for (priority, &quantum) in QUANTA.iter().enumerate() {
            let mut strategy = MlfqStrategy::new();
            strategy.remaining_quantum = quantum;

            for tick in 1..=quantum {
                let result = strategy.should_preempt();
                if tick < quantum {
                    assert!(!result, "priority {priority}: expected false at tick {tick}");
                } else {
                    assert!(result, "priority {priority}: expected true at tick {tick}");
                }
            }
        }
    }

    // === on_task_start tests ===

    #[test]
    fn on_task_start_resets_quantum_based_on_current_priority() {
        let mut strategy = MlfqStrategy::new();
        strategy.remaining_quantum = 0;
        strategy.current_priority = 0;
        strategy.on_task_start(make_handle(1, 0));
        assert_eq!(strategy.remaining_quantum, QUANTA[0]);
    }

    #[test]
    fn on_task_start_resets_quantum_for_different_priorities() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);

        for (priority, &quantum) in QUANTA.iter().enumerate() {
            strategy.remaining_quantum = 0;
            strategy.current_priority = priority;
            strategy.on_task_start(h);
            assert_eq!(strategy.remaining_quantum, quantum, "priority {priority}");
        }
    }

    // === full lifecycle tests ===

    #[test]
    fn full_lifecycle_push_pick_run_requeue_preempted() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);

        strategy.push_ready(h);
        let picked = strategy.pick_next().unwrap();
        strategy.record_yield(picked, YieldReason::Preempted);
        strategy.requeue_after_run(picked);

        assert_eq!(strategy.queues[1].len(), 1);
        let requeued = strategy.pick_next().unwrap();
        assert_eq!(requeued, h);
    }

    #[test]
    fn full_lifecycle_push_pick_run_requeue_voluntary() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(1, 0);

        strategy.push_ready(h);
        let picked = strategy.pick_next().unwrap();
        assert_eq!(strategy.current_priority, 0);
        strategy.record_yield(picked, YieldReason::Voluntary);
        strategy.requeue_after_run(picked);

        // Voluntary yield keeps the same priority (queue 0)
        assert_eq!(strategy.queues[0].len(), 1);
        let requeued = strategy.pick_next().unwrap();
        assert_eq!(requeued, h);
    }

    #[test]
    fn multiple_tasks_priority_ordering_with_demotion() {
        let mut strategy = MlfqStrategy::new();
        let h1 = make_handle(1, 0);
        let h2 = make_handle(2, 0);

        strategy.push_ready(h1);
        strategy.push_ready(h2);

        // Pick h1 at priority 0, preempt it -> demotes to queue 1
        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h1);
        assert_eq!(strategy.current_priority, 0);
        strategy.record_yield(picked, YieldReason::Preempted);
        strategy.requeue_after_run(picked);

        // h2 is still in queue 0, h1 is in queue 1
        // Next pick should be h2 (higher priority queue)
        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h2);
        assert_eq!(strategy.current_priority, 0);
        strategy.record_yield(picked, YieldReason::Voluntary);
        strategy.requeue_after_run(picked);

        // Now h1 is in queue 1, h2 is in queue 0
        // Next pick should be h2 again (queue 0 > queue 1)
        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h2);
    }

    #[test]
    fn on_task_terminate_removes_handle_from_all_queues_and_yield_info() {
        let mut strategy = MlfqStrategy::new();
        let h1 = make_handle(1, 0);
        let h2 = make_handle(2, 0);

        strategy.push_ready(h1);
        strategy.push_ready(h2);

        // Pick h1, record yield (keep it in yield_info — don't requeue yet)
        let picked = strategy.pick_next().unwrap();
        assert_eq!(picked, h1);
        strategy.record_yield(picked, YieldReason::Preempted);

        // Push h1 directly into queue 1 to simulate a demoted task
        strategy.queues[1].push_back(h1);

        assert_eq!(strategy.queues[0].len(), 1); // h2
        assert_eq!(strategy.queues[1].len(), 1); // h1
        assert!(strategy.yield_info.contains_key(&h1));

        // Terminate h1
        strategy.on_task_terminate(h1);

        assert_eq!(strategy.queues[0].len(), 1);  // h2 remains
        assert_eq!(strategy.queues[1].len(), 0);  // h1 removed
        assert!(!strategy.yield_info.contains_key(&h1));
        assert_eq!(strategy.pick_next(), Some(h2));
    }

    #[test]
    fn on_task_terminate_on_nonexistent_handle_is_noop() {
        let mut strategy = MlfqStrategy::new();
        let h = make_handle(99, 0);
        strategy.on_task_terminate(h); // should not panic
        assert!(strategy.pick_next().is_none());
    }
}
