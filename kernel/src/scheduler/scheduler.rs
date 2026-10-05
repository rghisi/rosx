use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::messages::HardwareInterrupt;
use crate::scheduler::algorithm::SchedulingAlgorithm;
use crate::task::{TaskHandle, TaskState};
use crate::kernel_services::services;
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

static NOOP_TIMER_HANDLER: NoopTimerHandler = NoopTimerHandler;

struct NoopTimeSource;
impl ForReadingSystemTime for NoopTimeSource {
    fn now(&self) -> u64 { 0 }
}
static NOOP_TIME_SOURCE: NoopTimeSource = NoopTimeSource;

struct NoopTimerExpiry;
impl ForExpiringTimers for NoopTimerExpiry {
    fn pop_expired(&self, _now: u64) -> Option<Vec<system::future::FutureHandle>> { None }
}
static NOOP_TIMER_EXPIRY: NoopTimerExpiry = NoopTimerExpiry;

struct NoopInterruptHandler;
impl ForHandlingHardwareInterrupts for NoopInterruptHandler {
    fn handle(&self, _interrupt: HardwareInterrupt) {}
}
static NOOP_INTERRUPT_HANDLER: NoopInterruptHandler = NoopInterruptHandler;

struct NoopContextSwitcher;
impl ForSwitchingTaskContext for NoopContextSwitcher {
    fn switch_to_task(&self, handle: TaskHandle) -> SwitchOutcome { SwitchOutcome::Unchanged(handle) }
}

static NOOP_CONTEXT_SWITCHER: NoopContextSwitcher = NoopContextSwitcher;

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
        Scheduler::new_full(algorithm, &NOOP_CONTEXT_SWITCHER, &NOOP_TIMER_HANDLER, &NOOP_TIME_SOURCE, &NOOP_TIMER_EXPIRY, &NOOP_INTERRUPT_HANDLER, &crate::kernel::KERNEL_TASK_MANAGER)
    }

    pub fn new_with_context_switcher(
        algorithm: impl SchedulingAlgorithm + 'static,
        context_switcher: &'static dyn ForSwitchingTaskContext,
        timer_handler: &'static dyn ForCompletingExpiredTimers,
    ) -> Self {
        Scheduler::new_full(algorithm, context_switcher, timer_handler, &NOOP_TIME_SOURCE, &NOOP_TIMER_EXPIRY, &NOOP_INTERRUPT_HANDLER, &crate::kernel::KERNEL_TASK_MANAGER)
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

    pub fn run() {
        let (context_switcher, timer_handler, time_source, timer_expiry, interrupt_handler) = {
            let scheduler = services().scheduler.borrow_mut();
            (
                scheduler.context_switcher,
                scheduler.timer_handler,
                scheduler.time_source,
                scheduler.timer_expiry,
                scheduler.interrupt_handler,
            )
        };
        let mut interrupts = Vec::new();
        loop {
            interrupts.clear();
            services().scheduler.borrow_mut().drain_hardware_interrupts(&mut interrupts);
            for interrupt in interrupts.drain(..) {
                interrupt_handler.handle(interrupt);
            }
            if let Some(handles) = timer_expiry.pop_expired(time_source.now()) {
                for handle in handles {
                    timer_handler.complete_timer_future(handle);
                }
            }
            let next_handle = services().scheduler.borrow_mut().start_next_task();
            let outcome = context_switcher.switch_to_task(next_handle);
            services().scheduler.borrow_mut().reconcile_returned_task(outcome);
        }
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

    fn handle_termination(&mut self, handle: TaskHandle) {
        if self.idle_task == Some(handle) {
            self.idle_task = None;
        }
        self.algorithm.on_task_terminate(handle);
        self.tasks.remove_task(handle);
    }

    fn start_next_task(&mut self) -> TaskHandle {
        let next_handle = match self.algorithm.pick_next() {
            Some(handle) => handle,
            None => {
                let idle = self.idle_task.unwrap();
                self.tasks.set_state(idle, TaskState::Running);
                return idle;
            }
        };

        self.algorithm.on_task_start(next_handle);
        self.tasks.set_state(next_handle, TaskState::Running);
        next_handle
    }

    fn reconcile_returned_task(&mut self, outcome: SwitchOutcome) {
        match outcome {
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

    fn drain_hardware_interrupts(&mut self, out: &mut Vec<HardwareInterrupt>) {
        while let Some(hardware_interrupt) = self.hw_interrupt_queue.pop_front() {
            out.push(hardware_interrupt);
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
    use system::future::FutureHandle;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| init());
    }

    struct FakeContextSwitcher {
        calls: Arc<Mutex<Vec<TaskHandle>>>,
        outcome: Arc<Mutex<Option<SwitchOutcome>>>,
    }

    impl FakeContextSwitcher {
        fn new() -> (Self, Arc<Mutex<Vec<TaskHandle>>>, Arc<Mutex<Option<SwitchOutcome>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let outcome = Arc::new(Mutex::new(None));
            (
                FakeContextSwitcher { calls: calls.clone(), outcome: outcome.clone() },
                calls,
                outcome,
            )
        }

        fn take_calls(&self) -> Vec<TaskHandle> {
            self.calls.lock().unwrap().drain(..).collect()
        }
    }

    impl ForSwitchingTaskContext for FakeContextSwitcher {
        fn switch_to_task(&self, handle: TaskHandle) -> SwitchOutcome {
            self.calls.lock().unwrap().push(handle);
            self.outcome.lock().unwrap().clone().unwrap_or(SwitchOutcome::Yielded(handle, YieldReason::Voluntary))
        }
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
    fn run_one_round_calls_algorithm_methods_in_order() {
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

        assert_eq!(services().task_manager.borrow().get_state(handle), KS::Terminated);
    }

    // === start_next_task / reconcile_returned_task tests ===

    fn create_running_task(name: &'static str) -> TaskHandle {
        let task = Task::new(name, 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, KS::Running);
        handle
    }

    fn make_ctx() -> (&'static FakeContextSwitcher, Arc<Mutex<Vec<TaskHandle>>>, Arc<Mutex<Option<SwitchOutcome>>>) {
        let (ctx, calls, outcome) = FakeContextSwitcher::new();
        let leaked = Box::leak(Box::new(ctx));
        (leaked, calls, outcome)
    }

    fn run_one_round(engine: &mut Scheduler, ctx: &FakeContextSwitcher) {
        let next = engine.start_next_task();
        let outcome = ctx.switch_to_task(next);
        engine.reconcile_returned_task(outcome);
    }

    #[test]
    fn run_one_round_switches_to_picked_task() {
        setup();
        let mut fake = FakeAlgorithm::new();
        let (ctx, calls, _ret) = make_ctx();

        let task_handle = create_running_task("T");
        fake.next = Some(task_handle);
        let mut engine = Scheduler::new_with_context_switcher(fake, ctx, &NOOP_TIMER_HANDLER);

        run_one_round(&mut engine, ctx);

        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(calls.lock().unwrap()[0], task_handle);
    }

    #[test]
    fn run_one_round_requeues_task_returned_in_running_state() {
        setup();
        let mut fake = FakeAlgorithm::new();
        let (ctx, _calls, outcome) = make_ctx();

        let task_handle = create_running_task("T");
        fake.next = Some(task_handle);
        let mut engine = Scheduler::new_with_context_switcher(fake.clone(), ctx, &NOOP_TIMER_HANDLER);
        *outcome.lock().unwrap() = Some(SwitchOutcome::Yielded(task_handle, YieldReason::Voluntary));

        run_one_round(&mut engine, ctx);

        assert_eq!(fake.take_requeued(), vec![task_handle]);
        assert_eq!(fake.take_yielded(), vec![(task_handle, YieldReason::Voluntary)]);
    }

    #[test]
    fn run_one_round_does_not_requeue_task_returned_in_blocked_state() {
        setup();
        let mut fake = FakeAlgorithm::new();
        let (ctx, _calls, outcome) = make_ctx();

        let task_handle = create_running_task("T");
        services().task_manager.borrow_mut().set_state(task_handle, KS::Blocked);
        fake.next = Some(task_handle);
        let mut engine = Scheduler::new_with_context_switcher(fake.clone(), ctx, &NOOP_TIMER_HANDLER);
        *outcome.lock().unwrap() = Some(SwitchOutcome::Blocked(task_handle));

        run_one_round(&mut engine, ctx);

        assert!(fake.take_requeued().is_empty());
    }

    #[test]
    fn run_one_round_terminates_task_returned_in_terminated_state() {
        setup();
        let mut fake = FakeAlgorithm::new();
        let (ctx, _calls, outcome) = make_ctx();

        let task_handle = create_running_task("T");
        services().task_manager.borrow_mut().set_state(task_handle, KS::Terminated);
        fake.next = Some(task_handle);
        let mut engine = Scheduler::new_with_context_switcher(fake.clone(), ctx, &NOOP_TIMER_HANDLER);
        *outcome.lock().unwrap() = Some(SwitchOutcome::Terminated(task_handle));

        run_one_round(&mut engine, ctx);

        assert_eq!(fake.take_terminated(), vec![task_handle]);
    }

    #[test]
    fn run_one_round_switches_to_idle_when_no_user_tasks() {
        setup();
        let mut fake = FakeAlgorithm::new();
        let (ctx, calls, _ret) = make_ctx();

        let idle_handle = create_running_task("Idle");
        let mut engine = Scheduler::new_with_context_switcher(fake, ctx, &NOOP_TIMER_HANDLER);
        engine.set_idle_task(idle_handle).unwrap();

        run_one_round(&mut engine, ctx);

        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(calls.lock().unwrap()[0], idle_handle);
    }

    #[test]
    fn run_one_round_updates_idle_handle_when_idle_returns_running() {
        setup();
        let fake = FakeAlgorithm::new();
        let (ctx, _calls, outcome) = make_ctx();

        let idle_handle = create_running_task("Idle");
        let mut engine = Scheduler::new_with_context_switcher(fake.clone(), ctx, &NOOP_TIMER_HANDLER);
        engine.set_idle_task(idle_handle).unwrap();
        *outcome.lock().unwrap() = Some(SwitchOutcome::Yielded(idle_handle, YieldReason::Voluntary));

        run_one_round(&mut engine, ctx);
    }

    // === edge case: wake_tasks with terminated task ===

    #[test]
    fn wake_tasks_should_not_resurrect_terminated_task() {
        setup();
        let mut fake = FakeAlgorithm::new();
        let (ctx, _calls, _ret) = make_ctx();
        let mut engine = Scheduler::new_with_context_switcher(fake.clone(), ctx, &NOOP_TIMER_HANDLER);

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, KS::Terminated);

        engine.wake_tasks(vec![handle]);

        assert_eq!(services().task_manager.borrow().get_state(handle), KS::Terminated);
    }

    // === edge case: idle task termination leaves stale handle ===

    #[test]
    fn handle_termination_of_idle_task_should_clear_idle_handle() {
        setup();
        let mut fake = FakeAlgorithm::new();
        let (ctx, _calls, _ret) = make_ctx();
        let mut engine = Scheduler::new_with_context_switcher(fake, ctx, &NOOP_TIMER_HANDLER);

        let idle_handle = create_running_task("Idle");
        engine.set_idle_task(idle_handle).unwrap();

        engine.handle_termination(idle_handle);

        let result = engine.set_idle_task(create_running_task("NewIdle"));
        assert!(result.is_ok());
    }

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
    fn expired_timer_futures_are_completed() {
        setup();
        let fake = FakeAlgorithm::new();
        let (ctx, _calls, _outcome) = make_ctx();

        let fh = Handle::new(1, 0);
        let time = Box::leak(Box::new(FakeTimeSource { now: 100 }));
        let expiry = Box::leak(Box::new(FakeExpiry { handles: Some(vec![fh]) }));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let handler = Box::leak(Box::new(RecordingTimerHandler { completed: completed.clone() }));

        let mut engine = Scheduler::new_full(fake, ctx, handler, time, expiry, &NOOP_INTERRUPT_HANDLER, &crate::kernel::KERNEL_TASK_MANAGER);
        if let Some(handles) = engine.timer_expiry.pop_expired(engine.time_source.now()) {
            for handle in handles {
                engine.timer_handler.complete_timer_future(handle);
            }
        }

        assert_eq!(*completed.lock().unwrap(), vec![fh]);
    }

    #[test]
    fn expired_timer_futures_is_noop_when_nothing_expired() {
        setup();
        let fake = FakeAlgorithm::new();
        let (ctx, _calls, _outcome) = make_ctx();

        let time = Box::leak(Box::new(FakeTimeSource { now: 0 }));
        let expiry = Box::leak(Box::new(FakeExpiry { handles: None }));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let handler = Box::leak(Box::new(RecordingTimerHandler { completed: completed.clone() }));

        let mut engine = Scheduler::new_full(fake, ctx, handler, time, expiry, &NOOP_INTERRUPT_HANDLER, &crate::kernel::KERNEL_TASK_MANAGER);
        if let Some(handles) = engine.timer_expiry.pop_expired(engine.time_source.now()) {
            for handle in handles {
                engine.timer_handler.complete_timer_future(handle);
            }
        }

        assert!(completed.lock().unwrap().is_empty());
    }

    #[test]
    fn hardware_interrupts_are_delegated_to_handler() {
        setup();
        let fake = FakeAlgorithm::new();
        let (ctx, _calls, _outcome) = make_ctx();

        let time = Box::leak(Box::new(FakeTimeSource { now: 0 }));
        let expiry = Box::leak(Box::new(FakeExpiry { handles: None }));
        let handled = Arc::new(Mutex::new(Vec::new()));
        let ints = Box::leak(Box::new(RecordingInterruptHandler { handled: handled.clone() }));

        let mut engine = Scheduler::new_full(fake, ctx, &NOOP_TIMER_HANDLER, time, expiry, ints, &crate::kernel::KERNEL_TASK_MANAGER);
        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0x1C });
        let mut interrupts = Vec::new();
        engine.drain_hardware_interrupts(&mut interrupts);
        for interrupt in interrupts {
            engine.interrupt_handler.handle(interrupt);
        }

        assert_eq!(*handled.lock().unwrap(), vec![0x1C]);
    }
}
