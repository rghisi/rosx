use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use kernel::{switch_to_task, TASK_MANAGER};
use kprintln;
use messages::HardwareInterrupt;
use runnable::Runnable;
use task_queue::TaskQueue;
use task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use task_arena::TaskHandle;

pub struct MainThread {
    idle_task: Option<TaskHandle>,
    user_tasks: VecDeque<TaskHandle>,
    blocked_tasks: Vec<TaskHandle>,
    hw_interrupt_queue: Vec<HardwareInterrupt>,
}

impl MainThread {

    pub fn new() -> Self {
        MainThread {
            idle_task: None,
            user_tasks: VecDeque::with_capacity(5),
            blocked_tasks: Vec::with_capacity(5),
            hw_interrupt_queue: Vec::with_capacity(5),
        }
    }
}

impl MainThread {

    pub(crate) fn run(&mut self) {
        loop {
            self.process_hardware_interrupts();
            self.check_blocked_tasks();
            self.run_user_process();
        }
    }

    fn process_hardware_interrupts(&mut self) {
        while !self.hw_interrupt_queue.is_empty() {
            let hardware_interrupt = self.hw_interrupt_queue.remove(0);
            let event = match hardware_interrupt {
                HardwareInterrupt::Keyboard { scancode } => { (scancode & 0x7F) as char }
            };
        }
    }

    fn run_user_process(&mut self) {
        let mut next_task_option = self.user_tasks.pop_front();
        let next_task_handle = match next_task_option {
            None => { self.idle_task.clone().unwrap() }
            Some(next_task) => { next_task}
        };

        TASK_MANAGER.lock().borrow_mut().set_state(next_task_handle, Running);

        let returned_task_handle = switch_to_task(next_task_handle);

        let task_state = TASK_MANAGER.lock().borrow().get_state(returned_task_handle);

        match task_state {
            Created => {}
            Ready => {}
            Running => {
                TASK_MANAGER.lock().borrow_mut().set_state(returned_task_handle, Ready);
                if returned_task_handle != self.idle_task.clone().unwrap() {
                    let _ = self.user_tasks.push_back(returned_task_handle);
                } else {
                    self.idle_task = Some(returned_task_handle);
                }
            }
            Blocked => {
                self.blocked_tasks.push(returned_task_handle);
            }
            Terminated => {

            }
        }
    }

    pub(crate) fn push_task(&mut self, task_handle: TaskHandle) {
        let _ = match TASK_MANAGER.lock().borrow().get_state(task_handle) {
            Ready => self.user_tasks.push_back(task_handle),
            // Blocked => self.blocked_tasks.push(task),
            _ => return
        };
    }

    pub(crate) fn push_hardware_interrupt(&mut self, hardware_interrupt: HardwareInterrupt) {
        self.hw_interrupt_queue.push(hardware_interrupt);
    }

    pub(crate) fn set_idle_task(&mut self, idle_task_handle: TaskHandle) -> Result<(), ()> {
        if self.idle_task.is_none() {
            self.idle_task = Some(idle_task_handle);
            Ok(())
        } else {
            Err(())
        }
    }

    fn check_blocked_tasks(&mut self) {
        let blocked: Vec<TaskHandle> = self.blocked_tasks.drain(..).collect();
        for task_handle in blocked.into_iter() {
            match TASK_MANAGER.lock().borrow().get_state(task_handle) {
                Ready => { let _ = self.user_tasks.push_back(task_handle); }
                _ => self.blocked_tasks.push(task_handle)
            }
        }
    }
}
