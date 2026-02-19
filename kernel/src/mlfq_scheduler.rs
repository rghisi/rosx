use alloc::collections::VecDeque;
use crate::kernel_services::services;
use crate::messages::HardwareInterrupt;
use crate::syscall::switch_to_task;
use crate::task::TaskHandle;
use crate::task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use crate::task::YieldReason;
use system::future::FutureHandle;

const NUM_QUEUES: usize = 3;

pub struct MlfqScheduler {
    queues: [VecDeque<TaskHandle>; NUM_QUEUES],
    blocked_tasks: VecDeque<TaskFuture>,
    hw_interrupt_queue: VecDeque<HardwareInterrupt>,
    idle_task: Option<TaskHandle>,
}

struct TaskFuture {
    task_handle: TaskHandle,
    future_handle: FutureHandle,
}

impl TaskFuture {
    fn is_completed(&self) -> bool {
        services()
            .future_registry
            .borrow_mut()
            .get(self.future_handle)
            .unwrap_or(true)
    }
}

impl MlfqScheduler {
    pub fn new() -> Self {
        MlfqScheduler {
            queues: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
            blocked_tasks: VecDeque::new(),
            hw_interrupt_queue: VecDeque::new(),
            idle_task: None,
        }
    }

    pub(crate) fn run(&mut self) {
        loop {
            self.process_hardware_interrupts();
            self.poll_futures();
            self.run_next_task();
        }
    }

    pub(crate) fn push_task(&mut self, handle: TaskHandle) {
        match services().task_manager.borrow().get_state(handle) {
            Ready => self.queues[0].push_back(handle),
            _ => (),
        }
    }

    pub(crate) fn push_blocked(&mut self, task_handle: TaskHandle, future_handle: FutureHandle) {
        self.blocked_tasks.push_back(TaskFuture { task_handle, future_handle });
    }

    pub(crate) fn push_hardware_interrupt(&mut self, interrupt: HardwareInterrupt) {
        self.hw_interrupt_queue.push_back(interrupt);
    }

    pub(crate) fn set_idle_task(&mut self, handle: TaskHandle) -> Result<(), ()> {
        if self.idle_task.is_none() {
            self.idle_task = Some(handle);
            Ok(())
        } else {
            Err(())
        }
    }

    fn next_priority(current: usize, yield_reason: Option<YieldReason>) -> usize {
        match yield_reason {
            None => 0,
            Some(YieldReason::Voluntary) => current,
            Some(YieldReason::Preempted) => (current + 1).min(NUM_QUEUES - 1),
        }
    }

    pub(crate) fn take_next_handle(&mut self) -> Option<(TaskHandle, usize)> {
        for (priority, queue) in self.queues.iter_mut().enumerate() {
            if let Some(handle) = queue.pop_front() {
                return Some((handle, priority));
            }
        }
        None
    }

    pub(crate) fn requeue_after_run(&mut self, handle: TaskHandle, priority: usize) {
        let yield_reason = services().task_manager.borrow().get_yield_reason(handle);
        let new_priority = Self::next_priority(priority, yield_reason);
        self.queues[new_priority].push_back(handle);
    }

    fn run_next_task(&mut self) {
        let (next_handle, priority) = match self.take_next_handle() {
            Some((handle, priority)) => (handle, priority),
            None => (self.idle_task.unwrap(), 0),
        };

        services().task_manager.borrow_mut().set_state(next_handle, Running);
        let returned_handle = switch_to_task(next_handle);

        let task_state = services().task_manager.borrow().get_state(returned_handle);
        match task_state {
            Created | Ready => {}
            Running => {
                services().task_manager.borrow_mut().set_state(returned_handle, Ready);
                if Some(returned_handle) != self.idle_task {
                    self.requeue_after_run(returned_handle, priority);
                }
            }
            Blocked => {}
            Terminated => {
                services().task_manager.borrow_mut().remove_task(returned_handle);
            }
        }
    }

    fn process_hardware_interrupts(&mut self) {
        while let Some(interrupt) = self.hw_interrupt_queue.pop_front() {
            match interrupt {
                HardwareInterrupt::Keyboard { scancode } => {
                    if scancode & 0x80 == 0 {
                        if let Ok(key) = crate::keyboard::Key::from_scancode_set1(scancode) {
                            let event = crate::keyboard::KeyboardEvent::from_key(key);
                            if let Some(c) = event.char {
                                crate::keyboard::push_key(c);
                            }
                        }
                    }
                }
            }
        }
    }

    fn poll_futures(&mut self) {
        for _ in 0..self.blocked_tasks.len() {
            if let Some(task_future) = self.blocked_tasks.pop_front() {
                if task_future.is_completed() {
                    services()
                        .future_registry
                        .borrow_mut()
                        .remove(task_future.future_handle);
                    services()
                        .task_manager
                        .borrow_mut()
                        .set_state(task_future.task_handle, Ready);
                    self.queues[0].push_back(task_future.task_handle);
                } else {
                    self.blocked_tasks.push_back(task_future);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn queue_len(&self, priority: usize) -> usize {
        self.queues[priority].len()
    }

    #[cfg(test)]
    pub(crate) fn poll_futures_for_test(&mut self) {
        self.poll_futures();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::future::TaskCompletionFuture;
    use crate::kernel_services::{init, services};
    use crate::task::{Task, TaskState, next_id};
    use alloc::boxed::Box;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| init());
    }

    fn create_ready_task(name: &'static str) -> TaskHandle {
        let task = Task::new(next_id(), name, 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Ready);
        handle
    }

    #[test]
    fn next_priority_with_no_yield_reason_returns_queue_0() {
        assert_eq!(MlfqScheduler::next_priority(1, None), 0);
        assert_eq!(MlfqScheduler::next_priority(2, None), 0);
    }

    #[test]
    fn next_priority_with_voluntary_keeps_current_priority() {
        assert_eq!(MlfqScheduler::next_priority(0, Some(YieldReason::Voluntary)), 0);
        assert_eq!(MlfqScheduler::next_priority(1, Some(YieldReason::Voluntary)), 1);
        assert_eq!(MlfqScheduler::next_priority(2, Some(YieldReason::Voluntary)), 2);
    }

    #[test]
    fn next_priority_with_preempted_demotes_one_level() {
        assert_eq!(MlfqScheduler::next_priority(0, Some(YieldReason::Preempted)), 1);
        assert_eq!(MlfqScheduler::next_priority(1, Some(YieldReason::Preempted)), 2);
    }

    #[test]
    fn next_priority_with_preempted_at_lowest_stays_at_lowest() {
        assert_eq!(MlfqScheduler::next_priority(2, Some(YieldReason::Preempted)), 2);
    }

    #[test]
    fn push_task_places_task_in_queue_0() {
        setup();
        let mut scheduler = MlfqScheduler::new();
        let h = create_ready_task("T");

        scheduler.push_task(h);

        assert_eq!(scheduler.queue_len(0), 1);
        assert_eq!(scheduler.queue_len(1), 0);
        assert_eq!(scheduler.queue_len(2), 0);
    }

    #[test]
    fn take_next_handle_returns_from_highest_non_empty_queue() {
        setup();
        let mut scheduler = MlfqScheduler::new();
        let h_high = create_ready_task("High");
        let h_low = create_ready_task("Low");

        scheduler.push_task(h_high);
        let (taken, priority) = scheduler.take_next_handle().unwrap();
        services().task_manager.borrow_mut().set_yield_reason(taken, YieldReason::Preempted);
        scheduler.requeue_after_run(taken, priority);

        scheduler.push_task(h_low);

        let (next, next_priority) = scheduler.take_next_handle().unwrap();
        assert_eq!(next, h_low);
        assert_eq!(next_priority, 0);
    }

    #[test]
    fn requeue_after_voluntary_yield_keeps_same_priority() {
        setup();
        let mut scheduler = MlfqScheduler::new();
        let h = create_ready_task("T");
        scheduler.push_task(h);

        let (taken, priority) = scheduler.take_next_handle().unwrap();
        services().task_manager.borrow_mut().set_yield_reason(taken, YieldReason::Voluntary);
        scheduler.requeue_after_run(taken, priority);

        assert_eq!(scheduler.queue_len(0), 1);
        assert_eq!(scheduler.queue_len(1), 0);
    }

    #[test]
    fn requeue_after_preemption_demotes_to_next_level() {
        setup();
        let mut scheduler = MlfqScheduler::new();
        let h = create_ready_task("T");
        scheduler.push_task(h);

        let (taken, priority) = scheduler.take_next_handle().unwrap();
        services().task_manager.borrow_mut().set_yield_reason(taken, YieldReason::Preempted);
        scheduler.requeue_after_run(taken, priority);

        assert_eq!(scheduler.queue_len(0), 0);
        assert_eq!(scheduler.queue_len(1), 1);
        assert_eq!(scheduler.queue_len(2), 0);
    }

    #[test]
    fn requeue_at_lowest_priority_stays_at_lowest_when_preempted() {
        setup();
        let mut scheduler = MlfqScheduler::new();
        let h = create_ready_task("T");
        scheduler.push_task(h);

        let (taken, p) = scheduler.take_next_handle().unwrap();
        services().task_manager.borrow_mut().set_yield_reason(taken, YieldReason::Preempted);
        scheduler.requeue_after_run(taken, p);

        let (taken, p) = scheduler.take_next_handle().unwrap();
        services().task_manager.borrow_mut().set_yield_reason(taken, YieldReason::Preempted);
        scheduler.requeue_after_run(taken, p);

        let (taken, p) = scheduler.take_next_handle().unwrap();
        assert_eq!(p, 2);
        services().task_manager.borrow_mut().set_yield_reason(taken, YieldReason::Preempted);
        scheduler.requeue_after_run(taken, p);

        assert_eq!(scheduler.queue_len(2), 1);
        assert_eq!(scheduler.queue_len(0), 0);
        assert_eq!(scheduler.queue_len(1), 0);
    }

    #[test]
    fn poll_futures_places_unblocked_task_in_queue_0() {
        setup();
        let mut scheduler = MlfqScheduler::new();

        let waited_on = create_ready_task("WaitedOn");
        let future = Box::new(TaskCompletionFuture::new(waited_on));
        let future_handle = services().future_registry.borrow_mut().register(future).unwrap();

        let waiting_task = create_ready_task("Waiting");
        scheduler.push_blocked(waiting_task, future_handle);

        services().task_manager.borrow_mut().set_state(waited_on, TaskState::Terminated);

        scheduler.poll_futures_for_test();

        assert_eq!(scheduler.queue_len(0), 1);
        assert_eq!(scheduler.queue_len(1), 0);
        assert_eq!(scheduler.queue_len(2), 0);
    }

    #[test]
    fn take_next_returns_none_when_all_queues_empty() {
        let mut scheduler = MlfqScheduler::new();
        assert!(scheduler.take_next_handle().is_none());
    }
}
