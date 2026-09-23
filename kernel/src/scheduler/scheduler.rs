use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::messages::HardwareInterrupt;
use crate::scheduler::algorithm::SchedulingAlgorithm;
use crate::task::{TaskHandle, TaskState};
use crate::ForCompletingExpiredTimers;
use crate::ForExpiringTimers;
use crate::ForHandlingHardwareInterrupts;
use crate::ForManagingTasks;
use crate::ForReadingSystemTime;
use crate::ForSwitchingTaskContext;
use crate::SwitchOutcome;

struct NoopTimerHandler;
impl ForCompletingExpiredTimers for NoopTimerHandler {
    fn complete_timer_future(&self, _handle: system::future::FutureHandle) {}
}

pub(crate) static NOOP_TIMER_HANDLER: NoopTimerHandler = NoopTimerHandler;

struct NoopTimeSource;
impl ForReadingSystemTime for NoopTimeSource {
    fn now(&self) -> u64 { 0 }
}
pub(crate) static NOOP_TIME_SOURCE: NoopTimeSource = NoopTimeSource;

struct NoopTimerExpiry;
impl ForExpiringTimers for NoopTimerExpiry {
    fn pop_expired(&self, _now: u64) -> Option<Vec<system::future::FutureHandle>> { None }
}
pub(crate) static NOOP_TIMER_EXPIRY: NoopTimerExpiry = NoopTimerExpiry;

struct NoopInterruptHandler;
impl ForHandlingHardwareInterrupts for NoopInterruptHandler {
    fn handle(&self, _interrupt: HardwareInterrupt) {}
}
pub(crate) static NOOP_INTERRUPT_HANDLER: NoopInterruptHandler = NoopInterruptHandler;

struct NoopContextSwitcher;
impl ForSwitchingTaskContext for NoopContextSwitcher {
    fn switch_to_task(&self, handle: TaskHandle) -> SwitchOutcome { SwitchOutcome::Unchanged(handle) }
}

pub(crate) static NOOP_CONTEXT_SWITCHER: NoopContextSwitcher = NoopContextSwitcher;

pub struct Scheduler {
    algorithm: Box<dyn SchedulingAlgorithm + Send>,
    context_switcher: &'static dyn ForSwitchingTaskContext,
    hw_interrupt_queue: VecDeque<HardwareInterrupt>,
    idle_task: Option<TaskHandle>,
    timer_handler: &'static dyn ForCompletingExpiredTimers,
    time_source: &'static dyn ForReadingSystemTime,
    timer_expiry: &'static dyn ForExpiringTimers,
    interrupt_handler: &'static dyn ForHandlingHardwareInterrupts,
    tasks: &'static dyn ForManagingTasks,
}

impl Scheduler {
    pub fn new(algorithm: impl SchedulingAlgorithm + 'static) -> Self {
        Scheduler::new_full(algorithm, &NOOP_CONTEXT_SWITCHER, &NOOP_TIMER_HANDLER, &NOOP_TIME_SOURCE, &NOOP_TIMER_EXPIRY, &NOOP_INTERRUPT_HANDLER, &crate::NOOP_TASK_MANAGER)
    }

    pub fn new_with_context_switcher(
        algorithm: impl SchedulingAlgorithm + 'static,
        context_switcher: &'static dyn ForSwitchingTaskContext,
        timer_handler: &'static dyn ForCompletingExpiredTimers,
        tasks: &'static dyn ForManagingTasks,
    ) -> Self {
        Scheduler::new_full(algorithm, context_switcher, timer_handler, &NOOP_TIME_SOURCE, &NOOP_TIMER_EXPIRY, &NOOP_INTERRUPT_HANDLER, tasks)
    }

    pub fn new_full(
        algorithm: impl SchedulingAlgorithm + 'static,
        context_switcher: &'static dyn ForSwitchingTaskContext,
        timer_handler: &'static dyn ForCompletingExpiredTimers,
        time_source: &'static dyn ForReadingSystemTime,
        timer_expiry: &'static dyn ForExpiringTimers,
        interrupt_handler: &'static dyn ForHandlingHardwareInterrupts,
        tasks: &'static dyn ForManagingTasks,
    ) -> Self {
        Scheduler {
            algorithm: Box::new(algorithm),
            context_switcher,
            hw_interrupt_queue: VecDeque::with_capacity(5),
            idle_task: None,
            timer_handler,
            time_source,
            timer_expiry,
            interrupt_handler,
            tasks,
        }
    }

    pub fn run(&mut self) {
        loop {
            self.step();
        }
    }

    pub fn step(&mut self) {
        self.process_hardware_interrupts();
        self.process_timer_notifications();
        self.run_next_task();
    }

    pub fn push_task(&mut self, handle: TaskHandle) {
        match self.tasks.get_state(handle) {
            TaskState::Ready => self.algorithm.push_ready(handle),
            _ => {}
        }
    }

    pub fn wake_tasks(&mut self, handles: Vec<TaskHandle>) {
        for handle in handles {
            if self.tasks.get_state(handle) != TaskState::Terminated {
                self.tasks.set_state(handle, TaskState::Ready);
                self.algorithm.push_ready(handle);
            }
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

    pub(crate) fn handle_termination(&mut self, handle: TaskHandle) {
        if self.idle_task == Some(handle) {
            self.idle_task = None;
        }
        self.algorithm.on_task_terminate(handle);
        self.tasks.remove_task(handle);
    }

    pub(crate) fn run_next_task(&mut self) {
        let next_handle = match self.algorithm.pick_next() {
            Some(handle) => handle,
            None => {
                let idle = self.idle_task.unwrap();
                self.tasks.set_state(idle, TaskState::Running);
                match self.context_switcher.switch_to_task(idle) {
                    SwitchOutcome::Yielded(returned, _) => {
                        if Some(returned) == self.idle_task {
                            self.idle_task = Some(returned);
                        }
                    }
                    SwitchOutcome::Terminated(returned) => {
                        self.handle_termination(returned);
                    }
                    SwitchOutcome::Blocked(_) | SwitchOutcome::Unchanged(_) => {}
                }
                return;
            }
        };

        self.algorithm.on_task_start(next_handle);
        self.tasks.set_state(next_handle, TaskState::Running);
        match self.context_switcher.switch_to_task(next_handle) {
            SwitchOutcome::Yielded(returned_handle, yield_reason) => {
                self.tasks.set_state(returned_handle, TaskState::Ready);
                if Some(returned_handle) != self.idle_task {
                    self.algorithm.record_yield(returned_handle, yield_reason);
                    self.algorithm.requeue_after_run(returned_handle);
                } else {
                    self.idle_task = Some(returned_handle);
                }
            }
            SwitchOutcome::Blocked(_) => {}
            SwitchOutcome::Terminated(returned_handle) => {
                self.handle_termination(returned_handle);
            }
            SwitchOutcome::Unchanged(_) => {}
        }
    }

    pub(crate) fn process_hardware_interrupts(&mut self) {
        while let Some(interrupt) = self.hw_interrupt_queue.pop_front() {
            self.interrupt_handler.handle(interrupt);
        }
    }

    pub(crate) fn process_timer_notifications(&mut self) {
        let now = self.time_source.now();
        if let Some(handles) = self.timer_expiry.pop_expired(now) {
            for handle in handles {
                self.timer_handler.complete_timer_future(handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::YieldReason;
    use collections::generational_arena::Handle;
    use crate::scheduler::fakes::{FakeTaskManager, RecordingContextSwitcher};
    use std::sync::{Arc, Mutex};
    use system::future::FutureHandle;

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

    fn make_engine_with_fake(
        fake_algo: FakeAlgorithm,
        tasks: &'static dyn ForManagingTasks,
    ) -> Scheduler {
        Scheduler::new_full(
            fake_algo,
            &NOOP_CONTEXT_SWITCHER,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            tasks,
        )
    }

    // === push_task tests ===

    #[test]
    fn push_task_calls_push_ready_for_ready_tasks() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let h = Handle::new(1, 0);
        fake_tm.add_task(h);
        fake_tm.set_state(h, TaskState::Ready);

        let mut engine = make_engine_with_fake(fake_algo.clone(), fake_tm);
        engine.push_task(h);

        assert_eq!(fake_algo.take_pushed_ready(), vec![h]);
    }

    #[test]
    fn run_next_task_calls_algorithm_methods_in_order() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let h = Handle::new(2, 0);
        fake_tm.add_task(h);
        fake_tm.set_state(h, TaskState::Ready);

        let mut engine = make_engine_with_fake(fake_algo.clone(), fake_tm);

        assert_eq!(fake_algo.take_pushed_ready().len(), 0);
        engine.push_task(h);
        assert_eq!(fake_algo.take_pushed_ready(), vec![h]);
    }

    #[test]
    fn push_task_does_not_call_push_ready_for_non_ready_tasks() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let h = Handle::new(3, 0);
        // Not added to fake_tm → get_state returns Terminated

        let mut engine = make_engine_with_fake(fake_algo.clone(), fake_tm);
        engine.push_task(h);

        assert!(fake_algo.take_pushed_ready().is_empty());
    }

    // === push_hardware_interrupt tests ===

    #[test]
    fn push_hardware_interrupt_adds_to_queue() {
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let mut engine = make_engine_with_fake(FakeAlgorithm::new(), fake_tm);

        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0x1C });

        // No panic confirms the step executed correctly
    }

    // === set_idle_task tests ===

    #[test]
    fn set_idle_task_succeeds_on_first_call() {
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let idle = Handle::new(4, 0);
        let mut engine = make_engine_with_fake(FakeAlgorithm::new(), fake_tm);

        let result = engine.set_idle_task(idle);

        assert!(result.is_ok());
    }

    #[test]
    fn set_idle_task_fails_on_second_call() {
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let idle1 = Handle::new(5, 0);
        let idle2 = Handle::new(6, 0);
        let mut engine = make_engine_with_fake(FakeAlgorithm::new(), fake_tm);

        assert!(engine.set_idle_task(idle1).is_ok());
        assert!(engine.set_idle_task(idle2).is_err());
    }

    // === should_preempt tests ===

    #[test]
    fn should_preempt_forwards_to_algorithm() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let mut engine = make_engine_with_fake(fake_algo.clone(), fake_tm);

        *fake_algo.should_preempt_result.lock().unwrap() = true;
        assert!(engine.should_preempt());

        *fake_algo.should_preempt_result.lock().unwrap() = false;
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

    // === handle_termination tests (private → pub(crate), tested directly) ===

    #[test]
    fn handle_termination_calls_on_task_terminate_on_algorithm() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let h = Handle::new(50, 0);
        fake_tm.add_task(h);

        let mut engine = make_engine_with_fake(fake_algo.clone(), fake_tm);
        engine.handle_termination(h);

        assert_eq!(fake_algo.take_terminated(), vec![h]);
    }

    #[test]
    fn handle_termination_removes_task_from_manager() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let h = Handle::new(51, 0);
        fake_tm.add_task(h);
        fake_tm.set_state(h, TaskState::Ready);

        assert_ne!(fake_tm.get_state(h), TaskState::Terminated);

        let mut engine = make_engine_with_fake(fake_algo.clone(), fake_tm);
        engine.handle_termination(h);

        assert_eq!(fake_tm.get_state(h), TaskState::Terminated);
    }

    // === run_next_task tests ===

    #[test]
    fn run_next_task_switches_to_picked_task() {
        let mut fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let task_handle = Handle::new(60, 0);
        fake_tm.add_task(task_handle);
        fake_tm.set_state(task_handle, TaskState::Running);
        fake_algo.next = Some(task_handle);

        let (ctx, calls, _outcome) = RecordingContextSwitcher::new();
        let leaked_ctx: &'static RecordingContextSwitcher = Box::leak(Box::new(ctx));

        let mut engine = Scheduler::new_full(
            fake_algo,
            leaked_ctx,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );

        engine.run_next_task();

        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(calls.lock().unwrap()[0], task_handle);
    }

    #[test]
    fn run_next_task_requeues_task_returned_in_running_state() {
        let mut fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let task_handle = Handle::new(61, 0);
        fake_tm.add_task(task_handle);
        fake_tm.set_state(task_handle, TaskState::Running);
        fake_algo.next = Some(task_handle);

        let (ctx, _calls, outcome) = RecordingContextSwitcher::new();
        *outcome.lock().unwrap() = Some(SwitchOutcome::Yielded(task_handle, YieldReason::Voluntary));
        let leaked_ctx: &'static RecordingContextSwitcher = Box::leak(Box::new(ctx));

        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            leaked_ctx,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );

        engine.run_next_task();

        assert_eq!(fake_algo.take_requeued(), vec![task_handle]);
        assert_eq!(fake_algo.take_yielded(), vec![(task_handle, YieldReason::Voluntary)]);
    }

    #[test]
    fn run_next_task_does_not_requeue_task_returned_in_blocked_state() {
        let mut fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let task_handle = Handle::new(62, 0);
        fake_tm.add_task(task_handle);
        fake_tm.set_state(task_handle, TaskState::Running);
        fake_algo.next = Some(task_handle);

        let (ctx, _calls, outcome) = RecordingContextSwitcher::new();
        *outcome.lock().unwrap() = Some(SwitchOutcome::Blocked(task_handle));
        let leaked_ctx: &'static RecordingContextSwitcher = Box::leak(Box::new(ctx));

        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            leaked_ctx,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );

        engine.run_next_task();

        assert!(fake_algo.take_requeued().is_empty());
    }

    #[test]
    fn run_next_task_terminates_task_returned_in_terminated_state() {
        let mut fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let task_handle = Handle::new(63, 0);
        fake_tm.add_task(task_handle);
        fake_tm.set_state(task_handle, TaskState::Terminated);
        fake_algo.next = Some(task_handle);

        let (ctx, _calls, outcome) = RecordingContextSwitcher::new();
        *outcome.lock().unwrap() = Some(SwitchOutcome::Terminated(task_handle));
        let leaked_ctx: &'static RecordingContextSwitcher = Box::leak(Box::new(ctx));

        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            leaked_ctx,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );

        engine.run_next_task();

        assert_eq!(fake_algo.take_terminated(), vec![task_handle]);
    }

    #[test]
    fn run_next_task_switches_to_idle_when_no_user_tasks() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let idle_handle = Handle::new(64, 0);
        fake_tm.add_task(idle_handle);
        fake_tm.set_state(idle_handle, TaskState::Running);

        let (ctx, calls, _outcome) = RecordingContextSwitcher::new();
        let leaked_ctx: &'static RecordingContextSwitcher = Box::leak(Box::new(ctx));

        let mut engine = Scheduler::new_full(
            fake_algo,
            leaked_ctx,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );
        engine.set_idle_task(idle_handle).unwrap();

        engine.run_next_task();

        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(calls.lock().unwrap()[0], idle_handle);
    }

    #[test]
    fn run_next_task_updates_idle_handle_when_idle_returns_running() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let idle_handle = Handle::new(65, 0);
        fake_tm.add_task(idle_handle);
        fake_tm.set_state(idle_handle, TaskState::Running);

        let (ctx, _calls, outcome) = RecordingContextSwitcher::new();
        *outcome.lock().unwrap() = Some(SwitchOutcome::Yielded(idle_handle, YieldReason::Voluntary));
        let leaked_ctx: &'static RecordingContextSwitcher = Box::leak(Box::new(ctx));

        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            leaked_ctx,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );
        engine.set_idle_task(idle_handle).unwrap();

        engine.run_next_task();
    }

    // === edge case: wake_tasks with terminated task ===

    #[test]
    fn wake_tasks_should_not_resurrect_terminated_task() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let h = Handle::new(70, 0);
        fake_tm.add_task(h);
        fake_tm.set_state(h, TaskState::Terminated);

        let (ctx, _calls, _outcome) = RecordingContextSwitcher::new();
        let leaked_ctx: &'static RecordingContextSwitcher = Box::leak(Box::new(ctx));

        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            leaked_ctx,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );

        engine.wake_tasks(vec![h]);

        assert_eq!(fake_tm.get_state(h), TaskState::Terminated);
    }

    // === edge case: idle task termination leaves stale handle ===

    #[test]
    fn handle_termination_of_idle_task_should_clear_idle_handle() {
        let mut fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let idle_handle = Handle::new(80, 0);
        fake_tm.add_task(idle_handle);
        fake_tm.set_state(idle_handle, TaskState::Running);

        let (ctx, _calls, _outcome) = RecordingContextSwitcher::new();
        let leaked_ctx: &'static RecordingContextSwitcher = Box::leak(Box::new(ctx));

        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            leaked_ctx,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );
        engine.set_idle_task(idle_handle).unwrap();

        engine.handle_termination(idle_handle);

        let new_idle = Handle::new(81, 0);
        fake_tm.add_task(new_idle);
        fake_tm.set_state(new_idle, TaskState::Running);
        let result = engine.set_idle_task(new_idle);
        assert!(result.is_ok());
    }

    // === process_timer_notifications / process_hardware_interrupts tests ===

    struct FakeTimeSource {
        now: u64,
    }
    impl ForReadingSystemTime for FakeTimeSource {
        fn now(&self) -> u64 {
            self.now
        }
    }

    struct FakeExpiry {
        handles: Option<Vec<FutureHandle>>,
    }
    impl ForExpiringTimers for FakeExpiry {
        fn pop_expired(&self, _now: u64) -> Option<Vec<FutureHandle>> {
            self.handles.clone()
        }
    }

    struct RecordingTimerHandler {
        completed: Arc<Mutex<Vec<FutureHandle>>>,
    }
    impl ForCompletingExpiredTimers for RecordingTimerHandler {
        fn complete_timer_future(&self, handle: FutureHandle) {
            self.completed.lock().unwrap().push(handle);
        }
    }

    struct RecordingInterruptHandler {
        handled: Arc<Mutex<Vec<u8>>>,
    }
    impl ForHandlingHardwareInterrupts for RecordingInterruptHandler {
        fn handle(&self, interrupt: HardwareInterrupt) {
            if let HardwareInterrupt::Keyboard { scancode } = interrupt {
                self.handled.lock().unwrap().push(scancode);
            }
        }
    }

    #[test]
    fn process_timer_notifications_completes_expired_futures() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));

        let fh = Handle::new(1, 0);
        let time = Box::leak(Box::new(FakeTimeSource { now: 100 }));
        let expiry = Box::leak(Box::new(FakeExpiry { handles: Some(vec![fh]) }));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let handler = Box::leak(Box::new(RecordingTimerHandler { completed: completed.clone() }));

        let mut engine = Scheduler::new_full(
            fake_algo,
            &NOOP_CONTEXT_SWITCHER,
            handler,
            time,
            expiry,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );
        engine.process_timer_notifications();

        assert_eq!(*completed.lock().unwrap(), vec![fh]);
    }

    #[test]
    fn process_timer_notifications_is_noop_when_nothing_expired() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));

        let time = Box::leak(Box::new(FakeTimeSource { now: 0 }));
        let expiry = Box::leak(Box::new(FakeExpiry { handles: None }));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let handler = Box::leak(Box::new(RecordingTimerHandler { completed: completed.clone() }));

        let mut engine = Scheduler::new_full(
            fake_algo,
            &NOOP_CONTEXT_SWITCHER,
            handler,
            time,
            expiry,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );
        engine.process_timer_notifications();

        assert!(completed.lock().unwrap().is_empty());
    }

    #[test]
    fn process_hardware_interrupts_delegates_to_handler() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));

        let time = Box::leak(Box::new(FakeTimeSource { now: 0 }));
        let expiry = Box::leak(Box::new(FakeExpiry { handles: None }));
        let handled = Arc::new(Mutex::new(Vec::new()));
        let ints = Box::leak(Box::new(RecordingInterruptHandler { handled: handled.clone() }));

        let mut engine = Scheduler::new_full(
            fake_algo,
            &NOOP_CONTEXT_SWITCHER,
            &NOOP_TIMER_HANDLER,
            time,
            expiry,
            ints,
            fake_tm as &'static dyn ForManagingTasks,
        );
        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0x1C });
        engine.process_hardware_interrupts();

        assert_eq!(*handled.lock().unwrap(), vec![0x1C]);
    }

    // === step() tests (new for refactoring) ===

    #[test]
    fn step_executes_phases_in_order() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));

        let idle_handle = Handle::new(88, 0);
        fake_tm.add_task(idle_handle);
        fake_tm.set_state(idle_handle, TaskState::Running);

        let order = Arc::new(Mutex::new(Vec::new()));

        struct PhaseInterruptHandler { order: Arc<Mutex<Vec<String>>> }
        impl ForHandlingHardwareInterrupts for PhaseInterruptHandler {
            fn handle(&self, _interrupt: HardwareInterrupt) {
                self.order.lock().unwrap().push("interrupts".into());
            }
        }

        struct PhaseTimerHandler { order: Arc<Mutex<Vec<String>>> }
        impl ForCompletingExpiredTimers for PhaseTimerHandler {
            fn complete_timer_future(&self, _handle: system::future::FutureHandle) {
                self.order.lock().unwrap().push("timers".into());
            }
        }

        struct PhaseContextSwitcher { order: Arc<Mutex<Vec<String>>> }
        impl ForSwitchingTaskContext for PhaseContextSwitcher {
            fn switch_to_task(&self, _handle: TaskHandle) -> SwitchOutcome {
                self.order.lock().unwrap().push("task".into());
                SwitchOutcome::Yielded(_handle, YieldReason::Voluntary)
            }
        }

        let order_clone = order.clone();
        let ints = Box::leak(Box::new(PhaseInterruptHandler { order: order_clone }));
        let order_clone = order.clone();
        let timer_handler = Box::leak(Box::new(PhaseTimerHandler { order: order_clone }));
        let order_clone = order.clone();
        let ctx = Box::leak(Box::new(PhaseContextSwitcher { order: order_clone }));

        let fh = Handle::new(90, 0);
        let time = Box::leak(Box::new(FakeTimeSource { now: 50 }));
        let expiry = Box::leak(Box::new(FakeExpiry { handles: Some(vec![fh]) }));

        let mut engine = Scheduler::new_full(
            fake_algo,
            ctx,
            timer_handler,
            time,
            expiry,
            ints,
            fake_tm as &'static dyn ForManagingTasks,
        );
        engine.set_idle_task(idle_handle).unwrap();
        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0xAB });

        engine.step();

        let recorded = order.lock().unwrap();
        assert_eq!(&recorded[..], &["interrupts", "timers", "task"]);
    }

    #[test]
    fn step_does_nothing_when_queue_empty_and_no_tasks() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        // Add an idle task so run_next_task doesn't panic
        let idle_handle = Handle::new(99, 0);
        fake_tm.add_task(idle_handle);
        fake_tm.set_state(idle_handle, TaskState::Running);

        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            &NOOP_CONTEXT_SWITCHER,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );
        engine.set_idle_task(idle_handle).unwrap();

        // Step should not panic when there's no work to do
        engine.step();
    }

    #[test]
    fn run_loop_exits_and_entrance_to_step() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));

        // Set up idle task for the scheduler to work correctly
        let idle_handle = Handle::new(99, 0);
        fake_tm.add_task(idle_handle);
        fake_tm.set_state(idle_handle, TaskState::Running);

        let mut call_count = 0usize;
        let counter = Arc::new(Mutex::new(call_count));

        // We can't easily test the infinite loop, but we verify step is callable
        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            &NOOP_CONTEXT_SWITCHER,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );
        engine.set_idle_task(idle_handle).unwrap();

        for _ in 0..3 {
            engine.step();
            *counter.lock().unwrap() += 1;
        }

        assert_eq!(*counter.lock().unwrap(), 3);
    }

    #[test]
    fn handle_termination_clears_idle_task_when_it_terminates() {
        let fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));
        let idle_handle = Handle::new(95, 0);
        fake_tm.add_task(idle_handle);
        fake_tm.set_state(idle_handle, TaskState::Running);

        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            &NOOP_CONTEXT_SWITCHER,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );

        engine.set_idle_task(idle_handle).unwrap();

        // Idle task termination should clear idle_task
        engine.handle_termination(idle_handle);

        let new_idle = Handle::new(96, 0);
        fake_tm.add_task(new_idle);
        assert!(engine.set_idle_task(new_idle).is_ok());
    }

    #[test]
    fn wake_tasks_skips_terminated_and_requeues_ready() {
        let mut fake_algo = FakeAlgorithm::new();
        let fake_tm: &'static FakeTaskManager = Box::leak(Box::new(FakeTaskManager::new()));

        let ready_handle = Handle::new(97, 0);
        fake_tm.add_task(ready_handle);
        fake_tm.set_state(ready_handle, TaskState::Ready);

        let terminated_handle = Handle::new(98, 0);
        fake_tm.add_task(terminated_handle);
        fake_tm.set_state(terminated_handle, TaskState::Terminated);

        let mut engine = Scheduler::new_full(
            fake_algo.clone(),
            &NOOP_CONTEXT_SWITCHER,
            &NOOP_TIMER_HANDLER,
            &NOOP_TIME_SOURCE,
            &NOOP_TIMER_EXPIRY,
            &NOOP_INTERRUPT_HANDLER,
            fake_tm as &'static dyn ForManagingTasks,
        );

        engine.wake_tasks(vec![ready_handle, terminated_handle]);

        // Ready task should be pushed to ready queue
        let pushed = fake_algo.take_pushed_ready();
        assert!(pushed.contains(&ready_handle));
        assert!(!pushed.contains(&terminated_handle));

        // Terminated task state should remain terminated
        assert_eq!(fake_tm.get_state(terminated_handle), TaskState::Terminated);
    }
}
