use alloc::vec::Vec;
use cpu::Cpu;
use wrappers::task_wrapper;
use kprintln;
use task_scheduler::TaskScheduler;
use task_queue::TaskQueue;
use task_fifo_queue::TaskFifoQueue;
use state::{CURRENT_TASK, MAIN_THREAD_TASK};
use task::{SharedTask, Task};
use task::TaskState::{Blocked, Created, Ready, Running};

pub struct RoundRobin {
    cpu: &'static dyn Cpu,
    idle_task: Option<SharedTask>,
    idle_task_pid: u32,
    ready_tasks: TaskFifoQueue,
    blocked_tasks: Vec<SharedTask>,
}

impl RoundRobin {

    pub fn new(cpu: &'static (dyn Cpu + 'static), mut idle_task: SharedTask) -> Self {

        let idle_task_pid = idle_task.id();
        let new_stack_pointer = cpu.initialize_task(
            idle_task.stack_pointer(),
            idle_task.entry_point(),
            idle_task.actual_entry_point()
        );
        idle_task.set_stack_pointer(new_stack_pointer);
        idle_task.set_ready();

        RoundRobin {
            cpu,
            idle_task: Some(idle_task),
            idle_task_pid,
            ready_tasks: TaskFifoQueue::new(),
            blocked_tasks: Vec::with_capacity(5),
        }
    }
}

impl TaskScheduler for RoundRobin {
    fn run(&mut self) {
        self.cpu.enable_interrupts();

        loop {
            let mut task_option = self.ready_tasks.take_next();
            if task_option.is_none() {
                task_option = self.idle_task.take();
            }

            let mut task = task_option.unwrap();
            task.set_running();
            let task_stack_pointer = task.stack_pointer();

            unsafe {
                CURRENT_TASK = Some(task);
                let mut mtr = MAIN_THREAD_TASK.take().expect("Main Thread not available");
                let sp = mtr.stack_pointer_mut();
                MAIN_THREAD_TASK = Some(mtr);
                self.cpu.swap_context(sp, task_stack_pointer);
            }

            let mut task = unsafe {
                CURRENT_TASK.take().expect("Task should be returned from yield")
            };

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
}
