pub mod algorithm;
pub mod fifo_strategy;
pub mod mlfq_strategy;
mod scheduler;
pub mod timer;

use alloc::boxed::Box;
use crate::ForCompletingExpiredTimers;
use crate::ForSwitchingTaskContext;

pub use algorithm::SchedulingAlgorithm;
pub use scheduler::Scheduler;
pub use timer::TimerManager;
pub(crate) use crate::NoopTaskManager;
pub(crate) use crate::NOOP_TASK_MANAGER;

pub type SchedulerFactory = fn(&'static dyn ForSwitchingTaskContext, &'static dyn ForCompletingExpiredTimers) -> Box<Scheduler>;

pub fn mfq_scheduler(ctx: &'static dyn ForSwitchingTaskContext, timer: &'static dyn ForCompletingExpiredTimers) -> Box<Scheduler> {
    Box::new(Scheduler::new_full(
        mlfq_strategy::MlfqStrategy::new(),
        ctx,
        timer,
        &crate::kernel::KERNEL_TIME_SOURCE,
        &crate::kernel::KERNEL_TIMER_EXPIRY,
        &crate::kernel::KERNEL_INTERRUPT_HANDLER,
        &crate::kernel::KERNEL_TASK_MANAGER,
    ))
}

pub fn fifo_scheduler(ctx: &'static dyn ForSwitchingTaskContext, timer: &'static dyn ForCompletingExpiredTimers) -> Box<Scheduler> {
    Box::new(Scheduler::new_full(
        fifo_strategy::FifoStrategy::new(),
        ctx,
        timer,
        &crate::kernel::KERNEL_TIME_SOURCE,
        &crate::kernel::KERNEL_TIMER_EXPIRY,
        &crate::kernel::KERNEL_INTERRUPT_HANDLER,
        &crate::kernel::KERNEL_TASK_MANAGER,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_services::{init, services};
    use crate::SwitchOutcome;
    use crate::task::{Task, TaskHandle, TaskState};
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| init());
    }

    struct TestContextSwitcher;
    impl ForSwitchingTaskContext for TestContextSwitcher {
        fn switch_to_task(&self, handle: TaskHandle) -> SwitchOutcome { SwitchOutcome::Unchanged(handle) }
    }
    static TEST_CTX: TestContextSwitcher = TestContextSwitcher;

    #[test]
    fn fifo_scheduler_factory_produces_functional_scheduler() {
        setup();
        let handler = services().timer_handler;
        let mut scheduler = *fifo_scheduler(&TEST_CTX, handler);

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);

        scheduler.wake_tasks(alloc::vec![handle]);
        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
    }

    #[test]
    fn mfq_scheduler_factory_produces_functional_scheduler() {
        setup();
        let handler = services().timer_handler;
        let mut scheduler = *mfq_scheduler(&TEST_CTX, handler);

        let task = Task::new("T", 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Blocked);

        scheduler.wake_tasks(alloc::vec![handle]);
        assert_eq!(services().task_manager.borrow().get_state(handle), TaskState::Ready);
    }

    #[test]
    fn services_expose_timer_handler() {
        setup();
        let handler = services().timer_handler;
        let _ = &*handler;
    }
}

#[cfg(test)]
pub(crate) mod fakes {
    use super::*;
    use crate::task::{TaskHandle, TaskState, YieldReason};
    use crate::{ForManagingTasks, SwitchOutcome};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    pub struct FakeTaskManager {
        state: Arc<Mutex<BTreeMap<TaskHandle, TaskState>>>,
    }

    impl FakeTaskManager {
        pub fn new() -> Self {
            FakeTaskManager {
                state: Arc::new(Mutex::new(BTreeMap::new())),
            }
        }

        pub fn set_state(&self, handle: TaskHandle, state: TaskState) {
            self.state.lock().unwrap().insert(handle, state);
        }

        pub fn get_state(&self, handle: TaskHandle) -> TaskState {
            self.state.lock().unwrap().get(&handle).copied().unwrap_or(TaskState::Terminated)
        }

        pub fn add_task(&self, handle: TaskHandle) {
            self.state.lock().unwrap().entry(handle).or_insert(TaskState::Created);
        }

        pub fn remove_task(&self, handle: TaskHandle) {
            self.state.lock().unwrap().remove(&handle);
        }
    }

    impl Default for FakeTaskManager {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ForManagingTasks for FakeTaskManager {
        fn get_state(&self, handle: TaskHandle) -> TaskState {
            self.state.lock().unwrap().get(&handle).copied().unwrap_or(TaskState::Terminated)
        }

        fn set_state(&self, handle: TaskHandle, state: TaskState) {
            if self.state.lock().unwrap().contains_key(&handle) {
                self.state.lock().unwrap().insert(handle, state);
            }
        }

        fn remove_task(&self, handle: TaskHandle) {
            self.state.lock().unwrap().remove(&handle);
        }
    }

    pub struct RecordingContextSwitcher {
        pub calls: Arc<Mutex<Vec<TaskHandle>>>,
        pub outcome: Arc<Mutex<Option<SwitchOutcome>>>,
    }

    impl RecordingContextSwitcher {
        pub fn new() -> (Self, Arc<Mutex<Vec<TaskHandle>>>, Arc<Mutex<Option<SwitchOutcome>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let outcome = Arc::new(Mutex::new(None));
            (
                RecordingContextSwitcher { calls: calls.clone(), outcome: outcome.clone() },
                calls,
                outcome,
            )
        }

        pub fn take_calls(&self) -> Vec<TaskHandle> {
            self.calls.lock().unwrap().drain(..).collect()
        }
    }

    impl ForSwitchingTaskContext for RecordingContextSwitcher {
        fn switch_to_task(&self, handle: TaskHandle) -> SwitchOutcome {
            self.calls.lock().unwrap().push(handle);
            self.outcome.lock().unwrap().clone().unwrap_or(SwitchOutcome::Yielded(handle, YieldReason::Voluntary))
        }
    }
}