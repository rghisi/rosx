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

pub type SchedulerFactory = fn(&'static dyn ForSwitchingTaskContext, &'static dyn ForCompletingExpiredTimers) -> Box<Scheduler>;

pub fn mfq_scheduler(ctx: &'static dyn ForSwitchingTaskContext, timer: &'static dyn ForCompletingExpiredTimers) -> Box<Scheduler> {
    Box::new(Scheduler::new_full(
        mlfq_strategy::MlfqStrategy::new(),
        ctx,
        timer,
        &crate::kernel::KERNEL_TIME_SOURCE,
        &crate::kernel::KERNEL_TIMER_EXPIRY,
        &crate::kernel::KERNEL_INTERRUPT_HANDLER,
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