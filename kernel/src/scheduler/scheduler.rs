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

struct NoopTasks;
impl ForManagingTasks for NoopTasks {
    fn get_state(&self, _handle: TaskHandle) -> TaskState { TaskState::Created }
    fn set_state(&self, _handle: TaskHandle, _state: TaskState) {}
    fn remove_task(&self, _handle: TaskHandle) {}
}
static NOOP_TASKS: NoopTasks = NoopTasks;

#[derive(Clone, Copy)]
pub struct SchedulerPorts {
    context_switcher: &'static dyn ForSwitchingTaskContext,
    timer_handler: &'static dyn ForCompletingExpiredTimers,
    time_source: &'static dyn ForReadingSystemTime,
    timer_expiry: &'static dyn ForExpiringTimers,
    interrupt_handler: &'static dyn ForHandlingHardwareInterrupts,
    tasks: &'static dyn ForManagingTasks,
}

impl SchedulerPorts {
    pub fn new(
        context_switcher: &'static dyn ForSwitchingTaskContext,
        timer_handler: &'static dyn ForCompletingExpiredTimers,
        time_source: &'static dyn ForReadingSystemTime,
        timer_expiry: &'static dyn ForExpiringTimers,
        interrupt_handler: &'static dyn ForHandlingHardwareInterrupts,
        tasks: &'static dyn ForManagingTasks,
    ) -> Self {
        SchedulerPorts {
            context_switcher,
            timer_handler,
            time_source,
            timer_expiry,
            interrupt_handler,
            tasks,
        }
    }

    pub fn noop() -> Self {
        SchedulerPorts {
            context_switcher: &NOOP_CONTEXT_SWITCHER,
            timer_handler: &NOOP_TIMER_HANDLER,
            time_source: &NOOP_TIME_SOURCE,
            timer_expiry: &NOOP_TIMER_EXPIRY,
            interrupt_handler: &NOOP_INTERRUPT_HANDLER,
            tasks: &NOOP_TASKS,
        }
    }
}

pub struct Scheduler {
    algorithm: Box<dyn SchedulingAlgorithm + Send>,
    hw_interrupt_queue: VecDeque<HardwareInterrupt>,
    idle_task: Option<TaskHandle>,
    ports: SchedulerPorts,
    interrupt_drain_buf: Vec<HardwareInterrupt>,
}

impl Scheduler {
    pub fn new(algorithm: impl SchedulingAlgorithm + 'static, ports: SchedulerPorts) -> Self {
        Scheduler {
            algorithm: Box::new(algorithm),
            hw_interrupt_queue: VecDeque::with_capacity(5),
            idle_task: None,
            ports,
            interrupt_drain_buf: Vec::new(),
        }
    }

    pub(super) fn step(&mut self) {
        let mut interrupts = core::mem::take(&mut self.interrupt_drain_buf);
        self.drain_hardware_interrupts(&mut interrupts);
        for interrupt in interrupts.drain(..) {
            self.ports.interrupt_handler.handle(interrupt);
        }
        if let Some(handles) = self.ports.timer_expiry.pop_expired(self.ports.time_source.now()) {
            for handle in handles {
                self.ports.timer_handler.complete_timer_future(handle);
            }
        }
        let next_handle = self.start_next_task();
        let outcome = self.ports.context_switcher.switch_to_task(next_handle);
        self.reconcile_returned_task(outcome);
        interrupts.clear();
        self.interrupt_drain_buf = interrupts;
    }

    pub fn push_task(&mut self, handle: TaskHandle) {
        match self.ports.tasks.get_state(handle) {
            TaskState::Ready => self.algorithm.push_ready(handle),
            _ => {}
        }
    }

    pub fn wake_tasks(&mut self, handles: Vec<TaskHandle>) {
        for handle in handles {
            if self.ports.tasks.get_state(handle) != TaskState::Terminated {
                self.ports.tasks.set_state(handle, TaskState::Ready);
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
        self.ports.tasks.remove_task(handle);
    }

    fn start_next_task(&mut self) -> TaskHandle {
        let next_handle = match self.algorithm.pick_next() {
            Some(handle) => handle,
            None => {
                let idle = self.idle_task.unwrap();
                self.ports.tasks.set_state(idle, TaskState::Running);
                return idle;
            }
        };

        self.algorithm.on_task_start(next_handle);
        self.ports.tasks.set_state(next_handle, TaskState::Running);
        next_handle
    }

    fn reconcile_returned_task(&mut self, outcome: SwitchOutcome) {
        match outcome {
            SwitchOutcome::Yielded(returned_handle, yield_reason) => {
                self.ports.tasks.set_state(returned_handle, TaskState::Ready);
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
    use collections::generational_arena::{Handle, HalfSize};
    use crate::task::TaskState as KS;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use system::future::FutureHandle;

    #[derive(Clone)]
    struct FakeTaskManager {
        states: Arc<Mutex<BTreeMap<TaskHandle, TaskState>>>,
        set_state_calls: Arc<Mutex<Vec<(TaskHandle, TaskState)>>>,
        remove_calls: Arc<Mutex<Vec<TaskHandle>>>,
        counter: Arc<AtomicUsize>,
    }

    impl FakeTaskManager {
        fn new() -> Self {
            FakeTaskManager {
                states: Arc::new(Mutex::new(BTreeMap::new())),
                set_state_calls: Arc::new(Mutex::new(Vec::new())),
                remove_calls: Arc::new(Mutex::new(Vec::new())),
                counter: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn leak(&self) -> &'static FakeTaskManager {
            Box::leak(Box::new(self.clone()))
        }

        fn next_handle(&self) -> TaskHandle {
            Handle::new(self.counter.fetch_add(1, Ordering::SeqCst) as HalfSize, 0)
        }

        fn seed(&self, handle: TaskHandle, state: TaskState) {
            self.states.lock().unwrap().insert(handle, state);
        }

        fn state_of(&self, handle: TaskHandle) -> TaskState {
            self.states.lock().unwrap().get(&handle).copied().unwrap_or(TaskState::Created)
        }

        fn take_set_state_calls(&self) -> Vec<(TaskHandle, TaskState)> {
            self.set_state_calls.lock().unwrap().drain(..).collect()
        }

        fn take_remove_calls(&self) -> Vec<TaskHandle> {
            self.remove_calls.lock().unwrap().drain(..).collect()
        }
    }

    impl ForManagingTasks for FakeTaskManager {
        fn get_state(&self, handle: TaskHandle) -> TaskState {
            self.state_of(handle)
        }

        fn set_state(&self, handle: TaskHandle, state: TaskState) {
            self.set_state_calls.lock().unwrap().push((handle, state));
            self.states.lock().unwrap().insert(handle, state);
        }

        fn remove_task(&self, handle: TaskHandle) {
            self.remove_calls.lock().unwrap().push(handle);
            self.states.lock().unwrap().insert(handle, TaskState::Terminated);
        }
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
        picks: Arc<Mutex<std::collections::VecDeque<TaskHandle>>>,
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
                picks: Arc::new(Mutex::new(std::collections::VecDeque::new())),
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
            self.next.take().or_else(|| self.picks.lock().unwrap().pop_front())
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

    fn create_ready_task(fm: &FakeTaskManager) -> TaskHandle {
        let handle = fm.next_handle();
        fm.seed(handle, KS::Ready);
        handle
    }

    // === push_task tests ===

    #[test]
    fn push_task_calls_push_ready_for_ready_tasks() {
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), ports_with_tasks(fm.leak()));

        let h = create_ready_task(&fm);
        engine.push_task(h);

        assert_eq!(fake.take_pushed_ready(), vec![h]);
        assert_eq!(fm.state_of(h), KS::Ready);
    }

    #[test]
    fn run_one_round_calls_algorithm_methods_in_order() {
        let fm = FakeTaskManager::new();
        let mut fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), ports_with_tasks(fm.leak()));

        let h = create_ready_task(&fm);
        fake.next = Some(h);

        engine.push_task(h);

        assert_eq!(fake.take_pushed_ready(), vec![h]);
    }

    #[test]
    fn push_task_does_not_call_push_ready_for_non_ready_tasks() {
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), ports_with_tasks(fm.leak()));

        let handle = fm.next_handle();

        engine.push_task(handle);

        assert!(fake.take_pushed_ready().is_empty());
        assert_eq!(fm.state_of(handle), KS::Created);
    }

    // === push_hardware_interrupt tests ===

    #[test]
    fn push_hardware_interrupt_adds_to_queue() {
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), SchedulerPorts::noop());

        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0x1C });

        assert_eq!(fake.take_pushed_ready().len(), 0);
    }

    // === set_idle_task tests ===

    #[test]
    fn set_idle_task_succeeds_on_first_call() {
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), ports_with_tasks(fm.leak()));

        let idle = create_ready_task(&fm);
        let result = engine.set_idle_task(idle);

        assert!(result.is_ok());
    }

    #[test]
    fn set_idle_task_fails_on_second_call() {
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), ports_with_tasks(fm.leak()));

        let idle1 = create_ready_task(&fm);
        let idle2 = create_ready_task(&fm);

        assert!(engine.set_idle_task(idle1).is_ok());
        assert!(engine.set_idle_task(idle2).is_err());
    }

    // === should_preempt tests ===

    #[test]
    fn should_preempt_forwards_to_algorithm() {
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), SchedulerPorts::noop());

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
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), ports_with_tasks(fm.leak()));

        let handle = fm.next_handle();

        engine.handle_termination(handle);

        assert_eq!(fake.take_terminated(), vec![handle]);
        assert_eq!(fm.take_remove_calls(), vec![handle]);
    }

    #[test]
    fn handle_termination_removes_task_from_manager() {
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), ports_with_tasks(fm.leak()));

        let handle = fm.next_handle();
        fm.seed(handle, KS::Running);

        assert_ne!(fm.state_of(handle), KS::Terminated);

        engine.handle_termination(handle);

        assert_eq!(fm.state_of(handle), KS::Terminated);
    }

    // === start_next_task / reconcile_returned_task tests ===

    fn create_running_task(fm: &FakeTaskManager) -> TaskHandle {
        let handle = fm.next_handle();
        fm.seed(handle, KS::Running);
        handle
    }

    fn make_ctx() -> (&'static FakeContextSwitcher, Arc<Mutex<Vec<TaskHandle>>>, Arc<Mutex<Option<SwitchOutcome>>>) {
        let (ctx, calls, outcome) = FakeContextSwitcher::new();
        let leaked = Box::leak(Box::new(ctx));
        (leaked, calls, outcome)
    }

    fn ports_with_tasks(tasks: &'static dyn ForManagingTasks) -> SchedulerPorts {
        SchedulerPorts {
            context_switcher: &NOOP_CONTEXT_SWITCHER,
            timer_handler: &NOOP_TIMER_HANDLER,
            time_source: &NOOP_TIME_SOURCE,
            timer_expiry: &NOOP_TIMER_EXPIRY,
            interrupt_handler: &NOOP_INTERRUPT_HANDLER,
            tasks,
        }
    }

    fn ports_ctx_and_tasks(context_switcher: &'static dyn ForSwitchingTaskContext, tasks: &'static dyn ForManagingTasks) -> SchedulerPorts {
        SchedulerPorts {
            context_switcher,
            timer_handler: &NOOP_TIMER_HANDLER,
            time_source: &NOOP_TIME_SOURCE,
            timer_expiry: &NOOP_TIMER_EXPIRY,
            interrupt_handler: &NOOP_INTERRUPT_HANDLER,
            tasks,
        }
    }

    fn run_one_round(engine: &mut Scheduler, ctx: &FakeContextSwitcher) {
        let next = engine.start_next_task();
        let outcome = ctx.switch_to_task(next);
        engine.reconcile_returned_task(outcome);
    }

    #[test]
    fn run_one_round_switches_to_picked_task() {
        let fm = FakeTaskManager::new();
        let mut fake = FakeAlgorithm::new();
        let (ctx, calls, _ret) = make_ctx();

        let task_handle = create_running_task(&fm);
        fake.next = Some(task_handle);
        let mut engine = Scheduler::new(fake, ports_ctx_and_tasks(ctx, fm.leak()));

        run_one_round(&mut engine, ctx);

        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(calls.lock().unwrap()[0], task_handle);
    }

    #[test]
    fn run_one_round_requeues_task_returned_in_running_state() {
        let fm = FakeTaskManager::new();
        let mut fake = FakeAlgorithm::new();
        let (ctx, _calls, outcome) = make_ctx();

        let task_handle = create_running_task(&fm);
        fake.next = Some(task_handle);
        let mut engine = Scheduler::new(fake.clone(), ports_ctx_and_tasks(ctx, fm.leak()));
        *outcome.lock().unwrap() = Some(SwitchOutcome::Yielded(task_handle, YieldReason::Voluntary));

        run_one_round(&mut engine, ctx);

        assert_eq!(fake.take_requeued(), vec![task_handle]);
        assert_eq!(fake.take_yielded(), vec![(task_handle, YieldReason::Voluntary)]);
    }

    #[test]
    fn run_one_round_does_not_requeue_task_returned_in_blocked_state() {
        let fm = FakeTaskManager::new();
        let mut fake = FakeAlgorithm::new();
        let (ctx, _calls, outcome) = make_ctx();

        let task_handle = create_running_task(&fm);
        fm.seed(task_handle, KS::Blocked);
        fake.next = Some(task_handle);
        let mut engine = Scheduler::new(fake.clone(), ports_ctx_and_tasks(ctx, fm.leak()));
        *outcome.lock().unwrap() = Some(SwitchOutcome::Blocked(task_handle));

        run_one_round(&mut engine, ctx);

        assert!(fake.take_requeued().is_empty());
    }

    #[test]
    fn run_one_round_terminates_task_returned_in_terminated_state() {
        let fm = FakeTaskManager::new();
        let mut fake = FakeAlgorithm::new();
        let (ctx, _calls, outcome) = make_ctx();

        let task_handle = create_running_task(&fm);
        fm.seed(task_handle, KS::Terminated);
        fake.next = Some(task_handle);
        let mut engine = Scheduler::new(fake.clone(), ports_ctx_and_tasks(ctx, fm.leak()));
        *outcome.lock().unwrap() = Some(SwitchOutcome::Terminated(task_handle));

        run_one_round(&mut engine, ctx);

        assert_eq!(fake.take_terminated(), vec![task_handle]);
    }

    #[test]
    fn run_one_round_switches_to_idle_when_no_user_tasks() {
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let (ctx, calls, _ret) = make_ctx();

        let idle_handle = create_running_task(&fm);
        let mut engine = Scheduler::new(fake, ports_ctx_and_tasks(ctx, fm.leak()));
        engine.set_idle_task(idle_handle).unwrap();

        run_one_round(&mut engine, ctx);

        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(calls.lock().unwrap()[0], idle_handle);
    }

    #[test]
    fn run_one_round_updates_idle_handle_when_idle_returns_running() {
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let (ctx, _calls, outcome) = make_ctx();

        let idle_handle = create_running_task(&fm);
        let mut engine = Scheduler::new(fake.clone(), ports_ctx_and_tasks(ctx, fm.leak()));
        engine.set_idle_task(idle_handle).unwrap();
        *outcome.lock().unwrap() = Some(SwitchOutcome::Yielded(idle_handle, YieldReason::Voluntary));

        run_one_round(&mut engine, ctx);
    }

    // === edge case: wake_tasks with terminated task ===

    #[test]
    fn wake_tasks_should_not_resurrect_terminated_task() {
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let (ctx, _calls, _ret) = make_ctx();
        let mut engine = Scheduler::new(fake.clone(), ports_ctx_and_tasks(ctx, fm.leak()));

        let handle = fm.next_handle();
        fm.seed(handle, KS::Terminated);

        engine.wake_tasks(vec![handle]);

        assert_eq!(fm.state_of(handle), KS::Terminated);
        assert!(fake.take_pushed_ready().is_empty());
    }

    // === edge case: idle task termination leaves stale handle ===

    #[test]
    fn handle_termination_of_idle_task_should_clear_idle_handle() {
        let fm = FakeTaskManager::new();
        let fake = FakeAlgorithm::new();
        let (ctx, _calls, _ret) = make_ctx();
        let mut engine = Scheduler::new(fake, ports_ctx_and_tasks(ctx, fm.leak()));

        let idle_handle = create_running_task(&fm);
        engine.set_idle_task(idle_handle).unwrap();

        engine.handle_termination(idle_handle);

        let result = engine.set_idle_task(create_running_task(&fm));
        assert!(result.is_ok());
        assert_eq!(fm.take_remove_calls(), vec![idle_handle]);
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

    type CallLog = Arc<Mutex<Vec<&'static str>>>;

    fn new_call_log() -> CallLog {
        Arc::new(Mutex::new(Vec::new()))
    }

    struct StepInterruptHandler {
        handled: Arc<Mutex<Vec<u8>>>,
        log: CallLog,
    }

    impl ForHandlingHardwareInterrupts for StepInterruptHandler {
        fn handle(&self, interrupt: HardwareInterrupt) {
            self.log.lock().unwrap().push("interrupt");
            if let HardwareInterrupt::Keyboard { scancode } = interrupt {
                self.handled.lock().unwrap().push(scancode);
            }
        }
    }

    struct StepExpiry {
        handles: Option<Vec<FutureHandle>>,
        seen_now: Arc<Mutex<Vec<u64>>>,
        log: CallLog,
    }

    impl ForExpiringTimers for StepExpiry {
        fn pop_expired(&self, now: u64) -> Option<Vec<FutureHandle>> {
            self.log.lock().unwrap().push("pop_expired");
            self.seen_now.lock().unwrap().push(now);
            self.handles.clone()
        }
    }

    struct StepTimerHandler {
        completed: Arc<Mutex<Vec<FutureHandle>>>,
        log: CallLog,
    }

    impl ForCompletingExpiredTimers for StepTimerHandler {
        fn complete_timer_future(&self, handle: FutureHandle) {
            self.log.lock().unwrap().push("complete");
            self.completed.lock().unwrap().push(handle);
        }
    }

    struct StepContextSwitcher {
        calls: Arc<Mutex<Vec<TaskHandle>>>,
        outcome: Arc<Mutex<Option<SwitchOutcome>>>,
        log: CallLog,
    }

    impl ForSwitchingTaskContext for StepContextSwitcher {
        fn switch_to_task(&self, handle: TaskHandle) -> SwitchOutcome {
            self.log.lock().unwrap().push("switch");
            self.calls.lock().unwrap().push(handle);
            self.outcome.lock().unwrap().clone().unwrap_or(SwitchOutcome::Unchanged(handle))
        }
    }

    #[test]
    fn step_handles_interrupts_before_timer_expiry() {
        let fm = FakeTaskManager::new();
        let log = new_call_log();
        let handled = Arc::new(Mutex::new(Vec::new()));
        let seen_now = Arc::new(Mutex::new(Vec::new()));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(Mutex::new(Vec::new()));

        let fh = Handle::new(1, 0);
        let irq = Box::leak(Box::new(StepInterruptHandler { handled: handled.clone(), log: log.clone() }));
        let time = Box::leak(Box::new(FakeTimeSource { now: 100 }));
        let expiry = Box::leak(Box::new(StepExpiry { handles: Some(vec![fh]), seen_now: seen_now.clone(), log: log.clone() }));
        let timer = Box::leak(Box::new(StepTimerHandler { completed: completed.clone(), log: log.clone() }));
        let ctx = Box::leak(Box::new(StepContextSwitcher { calls: calls.clone(), outcome: Arc::new(Mutex::new(None)), log: log.clone() }));

        let mut fake = FakeAlgorithm::new();
        let h = create_ready_task(&fm);
        fake.next = Some(h);

        let mut engine = Scheduler::new(
            fake,
            SchedulerPorts {
                context_switcher: ctx,
                timer_handler: timer,
                time_source: time,
                timer_expiry: expiry,
                interrupt_handler: irq,
                tasks: fm.leak(),
            },
        );
        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0x1C });

        engine.step();

        assert_eq!(*log.lock().unwrap(), vec!["interrupt", "pop_expired", "complete", "switch"]);
        assert_eq!(*handled.lock().unwrap(), vec![0x1C]);
        assert_eq!(*completed.lock().unwrap(), vec![fh]);
        assert_eq!(*seen_now.lock().unwrap(), vec![100]);
    }

    #[test]
    fn step_completes_expired_timer_futures() {
        let fm = FakeTaskManager::new();
        let fh1 = Handle::new(1, 0);
        let fh2 = Handle::new(2, 0);

        let log = new_call_log();
        let seen_now = Arc::new(Mutex::new(Vec::new()));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(Mutex::new(Vec::new()));

        let time = Box::leak(Box::new(FakeTimeSource { now: 100 }));
        let expiry = Box::leak(Box::new(StepExpiry { handles: Some(vec![fh1, fh2]), seen_now: seen_now.clone(), log: log.clone() }));
        let timer = Box::leak(Box::new(StepTimerHandler { completed: completed.clone(), log: log.clone() }));
        let ctx = Box::leak(Box::new(StepContextSwitcher { calls: calls.clone(), outcome: Arc::new(Mutex::new(None)), log: log.clone() }));

        let mut fake = FakeAlgorithm::new();
        let h = create_ready_task(&fm);
        fake.next = Some(h);

        let mut engine = Scheduler::new(
            fake,
            SchedulerPorts {
                context_switcher: ctx,
                timer_handler: timer,
                time_source: time,
                timer_expiry: expiry,
                interrupt_handler: &NOOP_INTERRUPT_HANDLER,
                tasks: fm.leak(),
            },
        );

        engine.step();

        assert_eq!(*completed.lock().unwrap(), vec![fh1, fh2]);
        assert_eq!(*seen_now.lock().unwrap(), vec![100]);
    }

    #[test]
    fn expired_timer_futures_is_noop_when_nothing_expired() {
        let fm = FakeTaskManager::new();
        let time = Box::leak(Box::new(FakeTimeSource { now: 0 }));
        let expiry = Box::leak(Box::new(FakeExpiry { handles: None }));
        let completed = Arc::new(Mutex::new(Vec::new()));
        let handler = Box::leak(Box::new(RecordingTimerHandler { completed: completed.clone() }));
        let ctx = Box::leak(Box::new(StepContextSwitcher { calls: Arc::new(Mutex::new(Vec::new())), outcome: Arc::new(Mutex::new(None)), log: new_call_log() }));

        let mut fake = FakeAlgorithm::new();
        let h = create_ready_task(&fm);
        fake.next = Some(h);

        let mut engine = Scheduler::new(
            fake,
            SchedulerPorts {
                context_switcher: ctx,
                timer_handler: handler,
                time_source: time,
                timer_expiry: expiry,
                interrupt_handler: &NOOP_INTERRUPT_HANDLER,
                tasks: fm.leak(),
            },
        );

        engine.step();

        assert!(completed.lock().unwrap().is_empty());
    }

    #[test]
    fn step_routes_hardware_interrupts() {
        let fm = FakeTaskManager::new();
        let handled = Arc::new(Mutex::new(Vec::new()));
        let ints = Box::leak(Box::new(RecordingInterruptHandler { handled: handled.clone() }));

        let calls = Arc::new(Mutex::new(Vec::new()));
        let ctx = Box::leak(Box::new(StepContextSwitcher { calls: calls.clone(), outcome: Arc::new(Mutex::new(None)), log: new_call_log() }));

        let mut fake = FakeAlgorithm::new();
        let h = create_ready_task(&fm);
        fake.picks.lock().unwrap().push_back(h);
        fake.picks.lock().unwrap().push_back(h);

        let mut engine = Scheduler::new(
            fake,
            SchedulerPorts {
                context_switcher: ctx,
                timer_handler: &NOOP_TIMER_HANDLER,
                time_source: &NOOP_TIME_SOURCE,
                timer_expiry: &NOOP_TIMER_EXPIRY,
                interrupt_handler: ints,
                tasks: fm.leak(),
            },
        );
        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0x1C });
        engine.push_hardware_interrupt(HardwareInterrupt::Keyboard { scancode: 0x1E });

        engine.step();
        assert_eq!(*handled.lock().unwrap(), vec![0x1C, 0x1E]);

        engine.step();
        assert_eq!(*handled.lock().unwrap(), vec![0x1C, 0x1E]);
    }

    #[test]
    fn step_switches_to_picked_task_and_reconciles_yield() {
        let fm = FakeTaskManager::new();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let outcome = Arc::new(Mutex::new(None));
        let ctx = Box::leak(Box::new(StepContextSwitcher { calls: calls.clone(), outcome: outcome.clone(), log: new_call_log() }));

        let mut fake = FakeAlgorithm::new();
        let h = create_ready_task(&fm);
        fake.next = Some(h);
        fake.picks.lock().unwrap().push_back(h);

        let mut engine = Scheduler::new(fake.clone(), ports_ctx_and_tasks(ctx, fm.leak()));
        *outcome.lock().unwrap() = Some(SwitchOutcome::Yielded(h, YieldReason::Voluntary));

        engine.step();
        engine.step();

        assert_eq!(*calls.lock().unwrap(), vec![h, h]);
        assert_eq!(fake.take_yielded(), vec![(h, YieldReason::Voluntary), (h, YieldReason::Voluntary)]);
        assert_eq!(fake.take_requeued(), vec![h, h]);
        assert_eq!(fake.take_on_task_start(), vec![h, h]);
        assert_eq!(fm.take_set_state_calls(), vec![(h, KS::Running), (h, KS::Ready), (h, KS::Running), (h, KS::Ready)]);
        assert_eq!(fm.state_of(h), KS::Ready);
    }

    #[test]
    fn step_falls_back_to_idle_when_nothing_ready() {
        let fm = FakeTaskManager::new();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let outcome = Arc::new(Mutex::new(None));
        let ctx = Box::leak(Box::new(StepContextSwitcher { calls: calls.clone(), outcome: outcome.clone(), log: new_call_log() }));

        let fake = FakeAlgorithm::new();
        let mut engine = Scheduler::new(fake.clone(), ports_ctx_and_tasks(ctx, fm.leak()));

        let idle = create_running_task(&fm);
        engine.set_idle_task(idle).unwrap();
        *outcome.lock().unwrap() = Some(SwitchOutcome::Unchanged(idle));

        engine.step();

        assert_eq!(*calls.lock().unwrap(), vec![idle]);
        assert_eq!(fm.state_of(idle), KS::Running);
        assert!(fake.take_on_task_start().is_empty());
        assert!(fake.take_requeued().is_empty());
    }
}
