use crate::kernel::TASK_MANAGER;
use crate::messages::HardwareInterrupt;
use crate::syscall::switch_to_task;
use crate::task::TaskHandle;
use crate::task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use alloc::collections::VecDeque;
use system::future::FutureHandle;

pub struct MainThread {
    idle_task: Option<TaskHandle>,
    user_tasks: VecDeque<TaskHandle>,
    blocked_tasks: VecDeque<TaskFuture>,
    hw_interrupt_queue: VecDeque<HardwareInterrupt>,
}

impl Default for MainThread {
    fn default() -> Self {
        Self::new()
    }
}

impl MainThread {
    pub fn new() -> Self {
        MainThread {
            idle_task: None,
            user_tasks: VecDeque::with_capacity(5),
            blocked_tasks: VecDeque::with_capacity(5),
            hw_interrupt_queue: VecDeque::with_capacity(5),
        }
    }
}

impl MainThread {
    pub(crate) fn run(&mut self) {
        loop {
            self.process_hardware_interrupts();
            self.pool_futures();
            self.run_user_process();
        }
    }

    fn process_hardware_interrupts(&mut self) {
        while let Some(hardware_interrupt) = self.hw_interrupt_queue.pop_front() {
            match hardware_interrupt {
                HardwareInterrupt::Keyboard { scancode } => {
                    // Ignore break codes (key release) in PS/2 Set 1
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

        TASK_MANAGER
            .borrow_mut()
            .set_state(next_task_handle, Running);

        let returned_task_handle = switch_to_task(next_task_handle);

        let task_state = TASK_MANAGER.borrow().get_state(returned_task_handle);

        match task_state {
            Created => {}
            Ready => {}
            Running => {
                TASK_MANAGER
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
                TASK_MANAGER
                    .borrow_mut()
                    .remove_task(returned_task_handle);
            }
        }
    }

    pub(crate) fn push_task(&mut self, task_handle: TaskHandle) {
        match TASK_MANAGER.borrow().get_state(task_handle) {
            Ready => self.user_tasks.push_back(task_handle),
            // Blocked => self.blocked_tasks.push(task),
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

    fn pool_futures(&mut self) {
        for _ in 0..self.blocked_tasks.len() {
            if let Some(task_future) = self.blocked_tasks.pop_front() {
                if task_future.is_completed() {
                    crate::kernel::FUTURE_REGISTRY.remove(task_future.future_handle);
                    TASK_MANAGER
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

struct TaskFuture {
    task_handle: TaskHandle,
    future_handle: FutureHandle,
}

impl TaskFuture {
    fn is_completed(&self) -> bool {
        crate::kernel::FUTURE_REGISTRY.get(self.future_handle).unwrap_or(true)
    }
}
