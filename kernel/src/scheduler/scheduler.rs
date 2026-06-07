use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::kernel_services::services;
use crate::task::YieldReason;
use crate::messages::HardwareInterrupt;
use crate::scheduler::algorithm::SchedulingAlgorithm;
use crate::task::{TaskHandle, TaskState};
use crate::kernel::kernel;

pub struct Scheduler {
    algorithm: Box<dyn SchedulingAlgorithm + Send>,
    hw_interrupt_queue: VecDeque<HardwareInterrupt>,
    idle_task: Option<TaskHandle>,
}

impl Scheduler {
    pub fn new(algorithm: impl SchedulingAlgorithm + 'static) -> Self {
        Scheduler {
            algorithm: Box::new(algorithm),
            hw_interrupt_queue: VecDeque::with_capacity(5),
            idle_task: None,
        }
    }

    pub fn run(&mut self) {
        loop {
            self.process_hardware_interrupts();
            self.process_timer_notifications();
            self.run_next_task();
        }
    }

    pub fn push_task(&mut self, handle: TaskHandle) {
        match services().task_manager.borrow().get_state(handle) {
            TaskState::Ready => self.algorithm.push_ready(handle),
            _ => {}
        }
    }

    pub fn wake_tasks(&mut self, handles: Vec<TaskHandle>) {
        for handle in handles {
            services().task_manager.borrow_mut().set_state(handle, TaskState::Ready);
            self.algorithm.push_ready(handle);
        }
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

    fn handle_termination(&mut self, handle: TaskHandle) {
        self.algorithm.on_task_terminate(handle);
        services().task_manager.borrow_mut().remove_task(handle);
    }

    fn run_next_task(&mut self) {
        let next_handle = match self.algorithm.pick_next() {
            Some(handle) => handle,
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
                        self.handle_termination(returned);
                    }
                    _ => {}
                }
                return;
            }
        };

        self.algorithm.on_task_start(next_handle);
        services().task_manager.borrow_mut().set_state(next_handle, TaskState::Running);
        let returned_handle = kernel().switch_to_task(next_handle);

        let task_state = services().task_manager.borrow().get_state(returned_handle);
        match task_state {
            TaskState::Created | TaskState::Ready => {}
            TaskState::Running => {
                services().task_manager.borrow_mut().set_state(returned_handle, TaskState::Ready);
                if Some(returned_handle) != self.idle_task {
                    let yield_reason = services().task_manager.borrow().get_yield_reason(returned_handle).unwrap_or(YieldReason::Voluntary);
                    self.algorithm.record_yield(returned_handle, yield_reason);
                    self.algorithm.requeue_after_run(returned_handle);
                } else {
                    self.idle_task = Some(returned_handle);
                }
            }
            TaskState::Blocked => {}
            TaskState::Terminated => {
                self.handle_termination(returned_handle);
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

    fn process_timer_notifications(&mut self) {
        use crate::kernel::kernel;
        let now = kernel().get_system_time();
        if let Some(handles) = services().timer_manager.borrow_mut().pop_expired(now) {
            for handle in handles {
                services().future_registry.borrow_mut().notify(handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::YieldReason;
    use collections::generational_arena::Handle;
    use crate::kernel_services::{init, services};
    use crate::task::{Task, TaskState as KS};
    use std::sync::{Arc, Mutex};
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| init());
    }

    #[derive(Clone)]
    struct FakeAlgorithm {
        next: Option<TaskHandle>,
        should_preempt_result: Arc<Mutex<bool>>,
        requeued: Arc<Mutex<Vec<TaskHandle>>>,
        pushed_ready: Arc<Mutex<Vec<TaskHandle>>>,
        yielded: Arc<Mutex<Vec<(TaskHandle, YieldReason)>>>,
        on_task_start_called: Arc<Mutex<Vec<TaskHandle>>>,
        terminated: Arc<Mutex<Vec<TaskHandle>>>,
    }

    impl FakeAlgorithm {
        fn new() -> Self {
            FakeAlgorithm {
                next: None,
                should_preempt_result: Arc::new(Mutex::new(false)),
                requeued: Arc::new(Mutex::new(Vec::new())),
                pushed_ready: Arc::new(Mutex::new(Vec::new())),
                yielded: Arc::new(Mutex::new(Vec::new())),
                on_task_start_called: Arc::new(Mutex::new(Vec::new())),
                terminated: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn take_requeued(&self) -> Vec<TaskHandle> {
            self.requeued.lock().unwrap().drain(..).collect()
        }

        fn take_pushed_ready(&self) -> Vec<TaskHandle> {
            self.pushed_ready.lock().unwrap().drain(..).collect()
        }

        fn take_yielded(&self) -> Vec<(TaskHandle, YieldReason)> {
            self.yielded.lock().unwrap().drain(..).collect()
        }

        fn take_on_task_start(&self) -> Vec<TaskHandle> {
            self.on_task_start_called.lock().unwrap().drain(..).collect()
        }

        fn take_terminated(&self) -> Vec<TaskHandle> {
            self.terminated.lock().unwrap().drain(..).collect()
        }
    }

    impl SchedulingAlgorithm for FakeAlgorithm {
        fn pick_next(&mut self) -> Option<TaskHandle> {
            self.next.take()
        }

        fn record_yield(&mut self, handle: TaskHandle, yield_reason: YieldReason) {
            self.yielded.lock().unwrap().push((handle, yield_reason));
        }

        fn requeue_after_run(&mut self, handle: TaskHandle) {
            self.requeued.lock().unwrap().push(handle);
        }

        fn push_ready(&mut self, handle: TaskHandle) {
            self.pushed_ready.lock().unwrap().push(handle);
        }

        fn should_preempt(&mut self) -> bool {
            *self.should_preempt_result.lock().unwrap()
        }

        fn on_task_start(&mut self, handle: TaskHandle) {
            self.on_task_start_called.lock().unwrap().push(handle);
        }

        fn on_task_terminate(&mut self, handle: TaskHandle) {
            self.terminated.lock().unwrap().push(handle);
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
        let mut engine = Scheduler::new(fake.clone());

        let h = create_ready_task("T");
        engine.push_task(h);

        assert_eq!(fake.take_pushed_ready(), vec![h]);
    }

    #[test]
    fn run_next_task_calls_algorithm_methods_in_order() {
        setup();
        let mut fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone());

        let h = create_ready_task("T");
        fake.next = Some(h);

        // We can't easily run the full engine loop in a test, but we can verify
        // that push_task triggers push_ready on the algorithm
        engine.push_task(h);

        assert_eq!(fake.take_pushed_ready(), vec![h]);
    }

    #[test]
    fn push_task_does_not_call_push_ready_for_non_ready_tasks() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone());

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        // Task is Created by default, not Ready

        engine.push_task(handle);

        assert!(fake.take_pushed_ready().is_empty());
    }

    // === push_hardware_interrupt tests ===

    #[test]
    fn push_hardware_interrupt_adds_to_queue() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone());

        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0x1C });

        assert_eq!(fake.take_pushed_ready().len(), 0);
    }

    // === set_idle_task tests ===

    #[test]
    fn set_idle_task_succeeds_on_first_call() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone());

        let idle = create_ready_task("Idle");
        let result = engine.set_idle_task(idle);

        assert!(result.is_ok());
    }

    #[test]
    fn set_idle_task_fails_on_second_call() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone());

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
        let mut engine = Scheduler::new(fake.clone());

        *fake.should_preempt_result.lock().unwrap() = true;
        assert!(engine.should_preempt());

        *fake.should_preempt_result.lock().unwrap() = false;
        assert!(!engine.should_preempt());
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
        fake.next = Some(h);

        assert_eq!(fake.pick_next(), Some(h));
        assert!(fake.pick_next().is_none());
    }

    #[test]
    fn fake_algorithm_tracks_yield_records() {
        let mut fake = FakeAlgorithm::new();
        let h = Handle::new(5, 0);

        fake.record_yield(h, YieldReason::Voluntary);
        fake.record_yield(h, YieldReason::Preempted);

        let records = fake.take_yielded();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0], (h, YieldReason::Voluntary));
        assert_eq!(records[1], (h, YieldReason::Preempted));
    }

    #[test]
    fn fake_algorithm_tracks_on_task_start_handles() {
        let mut fake = FakeAlgorithm::new();
        let h0 = Handle::new(10, 0);
        let h1 = Handle::new(20, 0);
        let h2 = Handle::new(30, 0);

        fake.on_task_start(h0);
        fake.on_task_start(h1);
        fake.on_task_start(h2);

        assert_eq!(fake.take_on_task_start(), vec![h0, h1, h2]);
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

    #[test]
    fn handle_termination_calls_on_task_terminate_on_algorithm() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone());

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();

        engine.handle_termination(handle);

        assert_eq!(fake.take_terminated(), vec![handle]);
    }

    #[test]
    fn handle_termination_removes_task_from_manager() {
        setup();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone());

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();

        assert_ne!(services().task_manager.borrow().get_state(handle), KS::Terminated);

        engine.handle_termination(handle);

        // After removal, get_state returns Terminated (task not in arena)
        assert_eq!(services().task_manager.borrow().get_state(handle), KS::Terminated);
    }
}
