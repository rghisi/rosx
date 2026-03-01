use crate::kernel_services::services;
use crate::messages::HardwareInterrupt;
use crate::scheduler::Scheduler;
use crate::scheduler::timer::Timer;
use crate::task::TaskHandle;
use crate::task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use alloc::collections::VecDeque;
use system::future::FutureHandle;
use crate::kernel::kernel;

pub struct FifoScheduler {
    idle_task: Option<TaskHandle>,
    user_tasks: VecDeque<TaskHandle>,
    wake_queue: VecDeque<TaskHandle>,
    timer: Timer,
    hw_interrupt_queue: VecDeque<HardwareInterrupt>,
}

impl Default for FifoScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl FifoScheduler {
    pub fn new() -> Self {
        FifoScheduler {
            idle_task: None,
            user_tasks: VecDeque::with_capacity(5),
            wake_queue: VecDeque::with_capacity(5),
            timer: Timer::new(),
            hw_interrupt_queue: VecDeque::with_capacity(5),
        }
    }

    pub(crate) fn run(&mut self) {
        loop {
            self.process_hardware_interrupts();
            self.process_timers();
            self.process_wakeups();
            self.run_user_process();
        }
    }

    pub(crate) fn push_task(&mut self, task_handle: TaskHandle) {
        match services().task_manager.borrow().get_state(task_handle) {
            Ready => self.user_tasks.push_back(task_handle),
            _ => (),
        }
    }

    pub(crate) fn wake_task(&mut self, handle: TaskHandle) {
        self.wake_queue.push_back(handle);
    }

    pub(crate) fn push_hardware_interrupt(&mut self, hardware_interrupt: HardwareInterrupt) {
        self.hw_interrupt_queue.push_back(hardware_interrupt);
    }

    pub(crate) fn set_idle_task(&mut self, idle_task_handle: TaskHandle) -> Result<(), ()> {
        if self.idle_task.is_none() {
            self.idle_task = Some(idle_task_handle);
            Ok(())
        } else {
            Err(())
        }
    }

    fn process_timers(&mut self) {
        if let Some(expired) = self.timer.pop_expired(kernel().get_system_time()) {
            for handle in expired {
                services().future_registry.borrow_mut().complete(handle);
            }
        }
    }

    fn process_wakeups(&mut self) {
        while let Some(handle) = self.wake_queue.pop_front() {
            services().task_manager.borrow_mut().set_state(handle, Ready);
            self.user_tasks.push_back(handle);
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

    fn run_user_process(&mut self) {
        let next_task_option = self.user_tasks.pop_front();
        let next_task_handle = match next_task_option {
            None => self.idle_task.unwrap(),
            Some(next_task) => next_task,
        };

        services().task_manager
            .borrow_mut()
            .set_state(next_task_handle, Running);

        let returned_task_handle = kernel().switch_to_task(next_task_handle);

        let task_state = services().task_manager.borrow().get_state(returned_task_handle);

        match task_state {
            Created => {}
            Ready => {}
            Running => {
                services().task_manager
                    .borrow_mut()
                    .set_state(returned_task_handle, Ready);
                if returned_task_handle != self.idle_task.unwrap() {
                    self.user_tasks.push_back(returned_task_handle);
                } else {
                    self.idle_task = Some(returned_task_handle);
                }
            }
            Blocked => {}
            Terminated => {
                self.complete_task_future(returned_task_handle);
                self.cleanup_completion_future(returned_task_handle);
                services().task_manager
                    .borrow_mut()
                    .remove_task(returned_task_handle);
            }
        }
    }

    fn complete_task_future(&mut self, task_handle: TaskHandle) {
        let completion_future = services().task_manager.borrow().get_completion_future(task_handle);
        if let Some(future_handle) = completion_future {
            services().future_registry.borrow_mut().complete(future_handle);
        }
    }

    fn cleanup_completion_future(&mut self, task_handle: TaskHandle) {
        let completion_future = services().task_manager.borrow().get_completion_future(task_handle);
        if let Some(future_handle) = completion_future {
            let is_subscribed = services().future_registry.borrow().is_subscribed(future_handle);
            if !is_subscribed {
                services().future_registry.borrow_mut().consume(future_handle).ok();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn cleanup_completion_future_for_test(&mut self, handle: TaskHandle) {
        self.cleanup_completion_future(handle);
    }
}

impl Scheduler for FifoScheduler {
    fn run(&mut self) {
        FifoScheduler::run(self);
    }

    fn push_task(&mut self, handle: TaskHandle) {
        FifoScheduler::push_task(self, handle);
    }

    fn wake_task(&mut self, handle: TaskHandle) {
        FifoScheduler::wake_task(self, handle);
    }

    fn push_sleep(&mut self, now: u64, sleep: core::time::Duration, future_handle: FutureHandle) {
        self.timer.add_sleep(now, sleep, future_handle);
    }

    fn push_hardware_interrupt(&mut self, interrupt: HardwareInterrupt) {
        FifoScheduler::push_hardware_interrupt(self, interrupt);
    }

    fn set_idle_task(&mut self, handle: TaskHandle) -> Result<(), ()> {
        FifoScheduler::set_idle_task(self, handle)
    }

    fn should_preempt(&mut self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_services::{init, services};
    use crate::task::{Task, TaskState};
    use alloc::boxed::Box;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| init());
    }

    fn create_ready_task(name: &'static str) -> TaskHandle {
        let task = Task::new(name, 0x1000, 0);
        let handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        services().task_manager.borrow_mut().set_state(handle, TaskState::Ready);
        handle
    }

    #[test]
    fn orphaned_completion_future_is_removed_when_task_terminates() {
        setup();
        let mut scheduler = FifoScheduler::new();

        let task = Task::new("T", 0, 0);
        let task_handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        let future = Box::new(crate::future::TaskCompletionFuture::new(task_handle));
        let future_handle = services().future_registry.borrow_mut().register(future).unwrap();
        services().task_manager.borrow_mut().set_completion_future(task_handle, future_handle);
        services().task_manager.borrow_mut().set_state(task_handle, TaskState::Terminated);

        scheduler.cleanup_completion_future_for_test(task_handle);

        assert!(services().future_registry.borrow_mut().get(future_handle).is_none());
    }

    #[test]
    fn waited_on_completion_future_is_not_removed_when_task_terminates() {
        setup();
        let mut scheduler = FifoScheduler::new();

        let task = Task::new("T", 0, 0);
        let task_handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        let future = Box::new(crate::future::TaskCompletionFuture::new(task_handle));
        let future_handle = services().future_registry.borrow_mut().register(future).unwrap();
        services().task_manager.borrow_mut().set_completion_future(task_handle, future_handle);

        let waiter = create_ready_task("Waiter");
        services().future_registry.borrow_mut().subscribe(future_handle, waiter);

        services().task_manager.borrow_mut().set_state(task_handle, TaskState::Terminated);

        scheduler.cleanup_completion_future_for_test(task_handle);

        assert!(services().future_registry.borrow_mut().get(future_handle).is_some());
    }
}