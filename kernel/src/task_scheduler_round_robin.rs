use alloc::vec::Vec;
use cpu::Cpu;
use wrappers::task_wrapper;
use kprintln;
use task_scheduler::TaskScheduler;
use task_queue::TaskQueue;
use task_fifo_queue::TaskFifoQueue;
use state::{CURRENT_TASK, MAIN_THREAD_TASK_PTR};
use task::{SharedTask, Task};
use task::TaskState::{Blocked, Created, Ready, Running};

pub struct RoundRobin {
    cpu: &'static dyn Cpu,
    task: SharedTask,
    idle_task: Option<SharedTask>,
    idle_task_pid: u32,
    ready_tasks: TaskFifoQueue,
    blocked_tasks: Vec<SharedTask>,
}

impl RoundRobin {

    pub fn new(cpu: &'static (dyn Cpu + 'static), mut idle_task: SharedTask) -> Self {
        let main_task = Task::new(0, "Main Thread", task_wrapper as usize, 0);

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
            task: main_task,
            idle_task: Some(idle_task),
            idle_task_pid,
            ready_tasks: TaskFifoQueue::new(),
            blocked_tasks: Vec::with_capacity(5),
        }
    }
}

impl TaskScheduler for RoundRobin {
    fn run(&mut self) {
        unsafe {
            MAIN_THREAD_TASK_PTR = Some(&mut *self.task as *mut Task);
        }

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
            }

            self.cpu.swap_context(self.task.stack_pointer_mut(), task_stack_pointer);

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
