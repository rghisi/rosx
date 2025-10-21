use alloc::vec::Vec;
use cpu::Cpu;
use kernel::switch_to_task;
use kprintln;
use task_scheduler::TaskScheduler;
use task_queue::TaskQueue;
use task_fifo_queue::TaskFifoQueue;
use task::{SharedTask, Task};
use task::TaskState::{Blocked, Created, Ready, Running};

pub struct RoundRobin {
    idle_task: Option<SharedTask>,
    idle_task_pid: u32,
    ready_tasks: TaskFifoQueue,
}

impl RoundRobin {

    pub fn new() -> Self {
        RoundRobin {
            idle_task: None,
            idle_task_pid: 0,
            ready_tasks: TaskFifoQueue::new(),
        }
    }
}

impl TaskScheduler for RoundRobin {
    fn run(&mut self) {
        loop {
            let mut task_option = self.ready_tasks.take_next();
            if task_option.is_none() {
                task_option = self.idle_task.take();
            }

            let mut task = task_option.unwrap();
            task.set_running();
            task = switch_to_task(task);

            if task.state() == Running {
                task.set_ready();
                if task.id() != self.idle_task_pid {
                    let _ = self.ready_tasks.offer(task);
                } else {
                    self.idle_task = Some(task);
                }
            }
        }
    }

    fn push_task(&mut self, task: SharedTask) {
        let _ = match task.state() {
            Ready => self.ready_tasks.offer(task),
            // Blocked => self.blocked_tasks.push(task),
            _ => return
        };
    }

    fn set_idle_task(&mut self, task: SharedTask) -> Result<(), ()> {
        if self.idle_task.is_none() {
            self.idle_task_pid = task.id();
            self.idle_task = Some(task);
            Ok(())
        } else {
            Err(())
        }
    }
}
