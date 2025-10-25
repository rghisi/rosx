use alloc::boxed::Box;
use alloc::vec::Vec;
use kernel::switch_to_task;
use kprintln;
use messages::HardwareInterrupt;
use task_queue::TaskQueue;
use task::{SharedTask, Task};
use task::TaskState::{Blocked, Created, Ready, Running};

pub struct MainThread {
    idle_task: Option<SharedTask>,
    idle_task_pid: u32,
    user_thread_queue: Box<dyn TaskQueue>,
    hw_interrupt_queue: Vec<HardwareInterrupt>,
}

impl MainThread {

    pub fn new(user_thread_queue: Box<dyn TaskQueue>) -> Self {
        MainThread {
            idle_task: None,
            idle_task_pid: 0,
            user_thread_queue,
            hw_interrupt_queue: Vec::with_capacity(5),
        }
    }
}

impl MainThread {
    pub(crate) fn run(&mut self) {
        loop {
            //Hardware Interrupts - doesn't work on Release due to compiler optimization
            //needs checking
            while !self.hw_interrupt_queue.is_empty() {
                let hardware_interrupt = self.hw_interrupt_queue.remove(0);
                match hardware_interrupt {
                    HardwareInterrupt::Keyboard { .. } => {
                        kprintln!("Hardware interrupt {:?}", hardware_interrupt);
                    }
                }
            }

            //User Tasks
            let mut task_option = self.user_thread_queue.take_next();
            if task_option.is_none() {
                task_option = self.idle_task.take();
            }

            let mut task = task_option.unwrap();
            task.set_running();
            task = switch_to_task(task);

            if task.state() == Running {
                task.set_ready();
                if task.id() != self.idle_task_pid {
                    let _ = self.user_thread_queue.offer(task);
                } else {
                    self.idle_task = Some(task);
                }
            }
        }
    }

    pub(crate) fn push_task(&mut self, task: SharedTask) {
        let _ = match task.state() {
            Ready => self.user_thread_queue.offer(task),
            // Blocked => self.blocked_tasks.push(task),
            _ => return
        };
    }
    pub(crate) fn push_hardware_interrupt(&mut self, hardware_interrupt: HardwareInterrupt) {
        self.hw_interrupt_queue.push(hardware_interrupt);
    }

    pub(crate) fn set_idle_task(&mut self, task: SharedTask) -> Result<(), ()> {
        if self.idle_task.is_none() {
            self.idle_task_pid = task.id();
            self.idle_task = Some(task);
            Ok(())
        } else {
            Err(())
        }
    }
}
