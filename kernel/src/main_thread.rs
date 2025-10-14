use alloc::vec::Vec;
use cpu::Cpu;
use kprintln;
use runnable::Runnable;
use scheduler::Scheduler;
use simple_scheduler::SimpleScheduler;
use state::{CURRENT_TASK, MAIN_THREAD_TASK, MAIN_THREAD_TASK_PTR};
use task::{SharedTask, Task};
use task::TaskState::{Blocked, Created, Ready, Running};

pub(crate) struct MainThread {
    cpu: &'static dyn Cpu,
    idle_task: Option<SharedTask>,
    idle_task_pid: u32,
    ready_tasks: SimpleScheduler,
    blocked_tasks: Vec<SharedTask>,
}

impl MainThread {

    pub(crate) fn new(cpu: &'static (dyn Cpu + 'static), mut idle_task: SharedTask) -> Self {
        let idle_task_pid = idle_task.id();
        let new_stack_pointer = cpu.initialize_task(
            idle_task.stack_pointer(),
            idle_task.entry_point(),
            idle_task.actual_entry_point()
        );
        idle_task.set_stack_pointer(new_stack_pointer);
        idle_task.set_ready();

        MainThread {
            cpu,
            // task: main_task,
            idle_task: Some(idle_task),
            idle_task_pid,
            ready_tasks: SimpleScheduler::new(),
            blocked_tasks: Vec::with_capacity(5),
        }
    }

    pub(crate) fn push_task(&mut self, task: SharedTask) {
        let _ = match task.state() {
            Ready => self.ready_tasks.offer(task),
            // Blocked => self.blocked_tasks.push(task),
            _ => return
        };
    }
}

impl Runnable for MainThread {
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
                let mut main_thread_task = MAIN_THREAD_TASK.take().unwrap();
                let main_thread_stack_pointer = main_thread_task.stack_pointer_mut();
                MAIN_THREAD_TASK = Some(main_thread_task);
                self.cpu.swap_context(main_thread_stack_pointer, task_stack_pointer);
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
}
