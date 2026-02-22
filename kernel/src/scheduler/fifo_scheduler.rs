use crate::kernel_services::services;
use crate::messages::HardwareInterrupt;
use crate::scheduler::Scheduler;
use crate::task::TaskHandle;
use crate::task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use alloc::collections::VecDeque;
use system::future::FutureHandle;
use crate::future::TaskFuture;
use crate::kernel::kernel;

pub struct FifoScheduler {
    idle_task: Option<TaskHandle>,
    user_tasks: VecDeque<TaskHandle>,
    blocked_tasks: VecDeque<TaskFuture>,
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
            blocked_tasks: VecDeque::with_capacity(5),
            hw_interrupt_queue: VecDeque::with_capacity(5),
        }
    }

    pub(crate) fn run(&mut self) {
        loop {
            self.process_hardware_interrupts();
            self.pool_futures();
            self.run_user_process();
        }
    }

    pub(crate) fn push_task(&mut self, task_handle: TaskHandle) {
        match services().task_manager.borrow().get_state(task_handle) {
            Ready => self.user_tasks.push_back(task_handle),
            _ => (),
        }
    }

    pub(crate) fn push_hardware_interrupt(&mut self, hardware_interrupt: HardwareInterrupt) {
        self.hw_interrupt_queue.push_back(hardware_interrupt);
    }

    pub(crate) fn push_blocked(&mut self, task_handle: TaskHandle, future_handle: FutureHandle) {
        let task_future = TaskFuture {
            task_handle,
            future_handle,
        };
        self.blocked_tasks.push_back(task_future);
    }

    pub(crate) fn set_idle_task(&mut self, idle_task_handle: TaskHandle) -> Result<(), ()> {
        if self.idle_task.is_none() {
            self.idle_task = Some(idle_task_handle);
            Ok(())
        } else {
            Err(())
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
                services().task_manager
                    .borrow_mut()
                    .remove_task(returned_task_handle);
            }
        }
    }

    pub(crate) fn pool_futures(&mut self) {
        for _ in 0..self.blocked_tasks.len() {
            if let Some(task_future) = self.blocked_tasks.pop_front() {
                if task_future.is_completed() {
                    services().task_manager
                        .borrow_mut()
                        .set_state(task_future.task_handle, Ready);
                    self.user_tasks.push_back(task_future.task_handle);
                } else {
                    self.blocked_tasks.push_back(task_future);
                }
            }
        }
    }
}

impl Scheduler for FifoScheduler {
    fn run(&mut self) {
        FifoScheduler::run(self);
    }

    fn push_task(&mut self, handle: TaskHandle) {
        FifoScheduler::push_task(self, handle);
    }

    fn push_blocked(&mut self, task_handle: TaskHandle, future_handle: FutureHandle) {
        FifoScheduler::push_blocked(self, task_handle, future_handle);
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