pub mod algorithm;
pub mod fifo_strategy;
pub mod mlfq_strategy;
mod scheduler;
pub mod timer;

use alloc::boxed::Box;
use crate::ForCompletingExpiredTimers;

pub use algorithm::SchedulingAlgorithm;
pub use scheduler::Scheduler;
pub use timer::TimerManager;

pub type SchedulerFactory = fn(&'static dyn ForCompletingExpiredTimers) -> Box<Scheduler>;

pub fn mfq_scheduler(timer_handler: &'static dyn ForCompletingExpiredTimers) -> Box<Scheduler> {
    Box::new(Scheduler::new_with_timer_handler(mlfq_strategy::MlfqStrategy::new(), timer_handler))
}

pub fn fifo_scheduler(timer_handler: &'static dyn ForCompletingExpiredTimers) -> Box<Scheduler> {
    Box::new(Scheduler::new_with_timer_handler(fifo_strategy::FifoStrategy::new(), timer_handler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_services::{init, services};
    use crate::task::{Task, TaskState};
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| init());
    }

    #[test]
    fn fifo_scheduler_factory_produces_functional_scheduler() {
        setup();
        let handler = services().timer_handler;
        let mut scheduler = *fifo_scheduler(handler);

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
        let mut scheduler = *mfq_scheduler(handler);

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