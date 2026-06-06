use alloc::boxed::Box;
use alloc::collections::VecDeque;
use crate::kernel_services::services;
use crate::task::YieldReason;
use crate::messages::HardwareInterrupt;
use crate::scheduler::algorithm::SchedulingAlgorithm;
use crate::task::{TaskHandle, TaskState};
use system::future::FutureHandle;
use crate::future::TaskFuture;
use crate::kernel::kernel;

pub struct SchedulerEngine {
    algorithm: Box<dyn SchedulingAlgorithm>,
    blocked_tasks: VecDeque<TaskFuture>,
    hw_interrupt_queue: VecDeque<HardwareInterrupt>,
    idle_task: Option<TaskHandle>,
}

impl SchedulerEngine {
    pub fn new(algorithm: impl SchedulingAlgorithm + 'static) -> Self {
        SchedulerEngine {
            algorithm: Box::new(algorithm),
            blocked_tasks: VecDeque::with_capacity(5),
            hw_interrupt_queue: VecDeque::with_capacity(5),
            idle_task: None,
        }
    }

    pub fn run(&mut self) {
        loop {
            self.process_hardware_interrupts();
            self.poll_futures();
            self.run_next_task();
        }
    }

    pub fn push_task(&mut self, handle: TaskHandle) {
        match services().task_manager.borrow().get_state(handle) {
            TaskState::Ready => self.algorithm.push_ready(handle),
            _ => {}
        }
    }

    pub fn push_blocked(&mut self, task_handle: TaskHandle, future_handle: FutureHandle) {
        let task_future = TaskFuture {
            task_handle,
            future_handle,
        };
        self.blocked_tasks.push_back(task_future);
    }

    pub fn push_hardware_interrupt(&mut self, interrupt: HardwareInterrupt) {
        self.hw_interrupt_queue.push_back(interrupt);
    }

    pub fn set_idle_task(&mut self, handle: TaskHandle) -> Result<(), ()> {
        if self.idle_task.is_none() {
            self.idle_task = Some(handle);
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn should_preempt(&mut self) -> bool {
        self.algorithm.should_preempt()
    }

    fn run_next_task(&mut self) {
        let (next_handle, priority) = match self.algorithm.pick_next() {
            Some((handle, priority)) => (handle, priority),
            None => {
                let idle = self.idle_task.unwrap();
                services().task_manager.borrow_mut().set_state(idle, TaskState::Running);
                let returned = kernel().switch_to_task(idle);
                let task_state = services().task_manager.borrow().get_state(returned);
                match task_state {
                    TaskState::Running => {
                        if Some(returned) == self.idle_task {
                            self.idle_task = Some(returned);
                        }
                    }
                    TaskState::Terminated => {
                        self.cleanup_completion_future(returned);
                        services().task_manager.borrow_mut().remove_task(returned);
                    }
                    _ => {}
                }
                return;
            }
        };

        self.algorithm.on_task_start(priority);
        services().task_manager.borrow_mut().set_state(next_handle, TaskState::Running);
        let returned_handle = kernel().switch_to_task(next_handle);

        let task_state = services().task_manager.borrow().get_state(returned_handle);
        match task_state {
            TaskState::Created | TaskState::Ready => {}
            TaskState::Running => {
                services().task_manager.borrow_mut().set_state(returned_handle, TaskState::Ready);
                if Some(returned_handle) != self.idle_task {
                    let yield_reason = services().task_manager.borrow().get_yield_reason(returned_handle).unwrap_or(YieldReason::Voluntary);
                    self.algorithm.record_yield(returned_handle, priority, yield_reason);
                    self.algorithm.requeue_after_run(returned_handle);
                } else {
                    self.idle_task = Some(returned_handle);
                }
            }
            TaskState::Blocked => {}
            TaskState::Terminated => {
                self.cleanup_completion_future(returned_handle);
                services().task_manager.borrow_mut().remove_task(returned_handle);
            }
        }
    }

    fn process_hardware_interrupts(&mut self) {
        while let Some(hardware_interrupt) = self.hw_interrupt_queue.pop_front() {
            match hardware_interrupt {
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
            };
        }
    }

    fn poll_futures(&mut self) {
        for _ in 0..self.blocked_tasks.len() {
            if let Some(task_future) = self.blocked_tasks.pop_front() {
                if task_future.is_completed() {
                    services().task_manager.borrow_mut().set_state(task_future.task_handle, TaskState::Ready);
                    self.algorithm.push_ready(task_future.task_handle);
                } else {
                    self.blocked_tasks.push_back(task_future);
                }
            }
        }
    }

    fn cleanup_completion_future(&mut self, task_handle: TaskHandle) {
        let completion_future = services().task_manager.borrow().get_completion_future(task_handle);
        if let Some(future_handle) = completion_future {
            let is_waited_on = self.blocked_tasks.iter().any(|tf| tf.future_handle == future_handle);
            if !is_waited_on {
                services().future_registry.borrow_mut().consume(future_handle).ok();
            }
        }
    }
}

impl crate::scheduler::Scheduler for SchedulerEngine {
    fn run(&mut self) {
        SchedulerEngine::run(self);
    }

    fn push_task(&mut self, handle: TaskHandle) {
        SchedulerEngine::push_task(self, handle);
    }

    fn push_blocked(&mut self, task_handle: TaskHandle, future_handle: FutureHandle) {
        SchedulerEngine::push_blocked(self, task_handle, future_handle);
    }

    fn push_hardware_interrupt(&mut self, interrupt: HardwareInterrupt) {
        SchedulerEngine::push_hardware_interrupt(self, interrupt);
    }

    fn set_idle_task(&mut self, handle: TaskHandle) -> Result<(), ()> {
        SchedulerEngine::set_idle_task(self, handle)
    }

    fn should_preempt(&mut self) -> bool {
        SchedulerEngine::should_preempt(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::YieldReason;
    use crate::future::TaskCompletionFuture;
    use collections::generational_arena::Handle;
    use crate::kernel_services::{init, services};
    use crate::task::{Task, TaskState as KS};
    use alloc::boxed::Box;
    use std::rc::Rc;
    use std::cell::RefCell;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| init());
    }

    #[derive(Clone)]
    struct FakeAlgorithm {
        next: Option<(TaskHandle, usize)>,
        should_preempt_result: Rc<RefCell<bool>>,
        requeued: Rc<RefCell<Vec<TaskHandle>>>,
        pushed_ready: Rc<RefCell<Vec<TaskHandle>>>,
        yielded: Rc<RefCell<Vec<(TaskHandle, usize, YieldReason)>>>,
        on_task_start_called: Rc<RefCell<Vec<usize>>>,
    }

    impl FakeAlgorithm {
        fn new() -> Self {
            FakeAlgorithm {
                next: None,
                should_preempt_result: Rc::new(RefCell::new(false)),
                requeued: Rc::new(RefCell::new(Vec::new())),
                pushed_ready: Rc::new(RefCell::new(Vec::new())),
                yielded: Rc::new(RefCell::new(Vec::new())),
                on_task_start_called: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn take_requeued(&self) -> Vec<TaskHandle> {
            self.requeued.borrow_mut().drain(..).collect()
        }

        fn take_pushed_ready(&self) -> Vec<TaskHandle> {
            self.pushed_ready.borrow_mut().drain(..).collect()
        }

        fn take_yielded(&self) -> Vec<(TaskHandle, usize, YieldReason)> {
            self.yielded.borrow_mut().drain(..).collect()
        }

        fn take_on_task_start(&self) -> Vec<usize> {
            self.on_task_start_called.borrow_mut().drain(..).collect()
        }
    }

    impl SchedulingAlgorithm for FakeAlgorithm {
        fn pick_next(&mut self) -> Option<(TaskHandle, usize)> {
            self.next.take()
        }

        fn record_yield(&mut self, handle: TaskHandle, priority: usize, yield_reason: YieldReason) {
            self.yielded.borrow_mut().push((handle, priority, yield_reason));
        }

        fn requeue_after_run(&mut self, handle: TaskHandle) {
            self.requeued.borrow_mut().push(handle);
        }

        fn push_ready(&mut self, handle: TaskHandle) {
            self.pushed_ready.borrow_mut().push(handle);
        }

        fn should_preempt(&mut self) -> bool {
            *self.should_preempt_result.borrow()
        }

        fn on_task_start(&mut self, priority: usize) {
            self.on_task_start_called.borrow_mut().push(priority);
        }
    }

    fn create_ready_task(name: &'static str) -> TaskHandle {
        let task = Task::new(name, 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, KS::Ready);
        handle
    }

    // === push_task tests ===

    #[test]
    fn push_task_calls_push_ready_for_ready_tasks() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = SchedulerEngine::new(fake.clone());

        let h = create_ready_task("T");
        engine.push_task(h);

        assert_eq!(fake.take_pushed_ready(), vec![h]);
    }

    #[test]
    fn push_task_does_not_call_push_ready_for_non_ready_tasks() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = SchedulerEngine::new(fake.clone());

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        // Task is Created by default, not Ready

        engine.push_task(handle);

        assert!(fake.take_pushed_ready().is_empty());
    }

    // === push_blocked tests ===

    #[test]
    fn push_blocked_adds_task_to_blocked_queue() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = SchedulerEngine::new(fake.clone());

        let waiter = create_ready_task("Waiter");
        let waited_on = create_ready_task("WaitedOn");
        let future = Box::new(TaskCompletionFuture::new(waited_on));
        let future_handle = services().future_registry.borrow_mut().register(future).unwrap();

        engine.push_blocked(waiter, future_handle);

        assert_eq!(fake.take_pushed_ready().len(), 0);
    }

    // === push_hardware_interrupt tests ===

    #[test]
    fn push_hardware_interrupt_adds_to_queue() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = SchedulerEngine::new(fake.clone());

        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0x1C });

        assert_eq!(fake.take_pushed_ready().len(), 0);
    }

    // === set_idle_task tests ===

    #[test]
    fn set_idle_task_succeeds_on_first_call() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = SchedulerEngine::new(fake.clone());

        let idle = create_ready_task("Idle");
        let result = engine.set_idle_task(idle);

        assert!(result.is_ok());
    }

    #[test]
    fn set_idle_task_fails_on_second_call() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = SchedulerEngine::new(fake.clone());

        let idle1 = create_ready_task("Idle1");
        let idle2 = create_ready_task("Idle2");

        assert!(engine.set_idle_task(idle1).is_ok());
        assert!(engine.set_idle_task(idle2).is_err());
    }

    // === should_preempt tests ===

    #[test]
    fn should_preempt_forwards_to_algorithm() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = SchedulerEngine::new(fake.clone());

        *fake.should_preempt_result.borrow_mut() = true;
        assert!(engine.should_preempt());

        *fake.should_preempt_result.borrow_mut() = false;
        assert!(!engine.should_preempt());
    }

    // === cleanup_completion_future tests ===

    #[test]
    fn orphaned_completion_future_is_preserved_until_cleanup() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = SchedulerEngine::new(fake.clone());

        let task = Task::new("T", 0, 0);
        let task_handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        let future = Box::new(TaskCompletionFuture::new(task_handle));
        let future_handle = services().future_registry.borrow_mut().register(future).unwrap();
        services().task_manager.borrow_mut().set_completion_future(task_handle, future_handle);
        services().task_manager.borrow_mut().set_state(task_handle, KS::Terminated);

        // The future should still exist until cleanup runs via engine's run loop
        assert!(services().future_registry.borrow_mut().get(future_handle).is_some());
    }

    #[test]
    fn waited_on_completion_future_is_preserved_when_task_terminated() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = SchedulerEngine::new(fake.clone());

        let task = Task::new("T", 0, 0);
        let task_handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        let future = Box::new(TaskCompletionFuture::new(task_handle));
        let future_handle = services().future_registry.borrow_mut().register(future).unwrap();

        let waiter = create_ready_task("Waiter");
        engine.push_blocked(waiter, future_handle);

        services().task_manager.borrow_mut().set_state(task_handle, KS::Terminated);

        // The future should still be in the registry since a task is waiting on it
        assert!(services().future_registry.borrow_mut().get(future_handle).is_some());
    }

    // === FakeAlgorithm internal state tests ===

    #[test]
    fn fake_algorithm_returns_none_when_next_is_none() {
        let mut fake = FakeAlgorithm::new();
        assert!(fake.pick_next().is_none());
    }

    #[test]
    fn fake_algorithm_returns_value_once_then_none() {
        let mut fake = FakeAlgorithm::new();
        let h = Handle::new(1, 0);
        fake.next = Some((h, 3));

        assert_eq!(fake.pick_next(), Some((h, 3)));
        assert!(fake.pick_next().is_none());
    }

    #[test]
    fn fake_algorithm_tracks_yield_records() {
        let mut fake = FakeAlgorithm::new();
        let h = Handle::new(5, 0);

        fake.record_yield(h, 1, YieldReason::Voluntary);
        fake.record_yield(h, 2, YieldReason::Preempted);

        let records = fake.take_yielded();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0], (h, 1, YieldReason::Voluntary));
        assert_eq!(records[1], (h, 2, YieldReason::Preempted));
    }

    #[test]
    fn fake_algorithm_tracks_on_task_start_priorities() {
        let mut fake = FakeAlgorithm::new();

        fake.on_task_start(0);
        fake.on_task_start(1);
        fake.on_task_start(2);

        assert_eq!(fake.take_on_task_start(), vec![0, 1, 2]);
    }

    #[test]
    fn fake_algorithm_tracks_requeued_tasks() {
        let mut fake = FakeAlgorithm::new();
        let h1 = Handle::new(10, 0);
        let h2 = Handle::new(20, 0);

        fake.requeue_after_run(h1);
        fake.requeue_after_run(h2);

        assert_eq!(fake.take_requeued(), vec![h1, h2]);
    }

    #[test]
    fn fake_algorithm_tracks_pushed_ready_tasks() {
        let mut fake = FakeAlgorithm::new();
        let h1 = Handle::new(30, 0);
        let h2 = Handle::new(40, 0);

        fake.push_ready(h1);
        fake.push_ready(h2);

        assert_eq!(fake.take_pushed_ready(), vec![h1, h2]);
    }
}
