use alloc::boxed::Box;
use alloc::vec::Vec;
use cpu::Cpu;
use kprintln;
use runnable::Runnable;
use scheduler::Scheduler;
use simple_scheduler::SimpleScheduler;
use task::{Task, TaskEntryPoint};
use task::TaskState::{Blocked, Created, Ready, Running};

pub(crate) struct MainThread {
    cpu: &'static dyn Cpu,
    task: Box<Task>,
    idle_task: Box<Task>,
    ready_tasks: SimpleScheduler,
    blocked_tasks: Vec<Box<Task>>,
}

impl MainThread {

    pub(crate) fn new(cpu: &'static (dyn Cpu + 'static), idle_task: Box<Task>) -> Self {
        let main_task = Task::new(0, "Main Thread", 0);

        MainThread {
            cpu,
            task: main_task,
            idle_task,
            ready_tasks: SimpleScheduler::new(),
            blocked_tasks: Vec::with_capacity(5),
        }
    }

    pub(crate) fn push_task(&mut self, task: Box<Task>) {
        let _ = match task.state() {
            Ready => self.ready_tasks.offer(task),
            // Blocked => self.blocked_tasks.push(task),
            _ => return
        };
    }
}

impl Runnable for MainThread {
    fn run(&mut self) {
        kprintln!("[MAIN_THREAD] run() called!");
        loop {
            if let Some(mut task) = self.ready_tasks.take_next() {
                kprintln!("[MAIN_THREAD] Scheduling task: {}", task.name());
                task.set_running();

                // Swap context: save MainThread's context, load task's context
                self.cpu.swap_context(self.task.stack_pointer_mut(), task.stack_pointer());

                // When we return here, the task has completed or yielded
                kprintln!("[MAIN_THREAD] Task {} completed/yielded", task.name());
                task.set_terminated();
            } else {
                kprintln!("[MAIN_THREAD] No ready tasks, running idle task");

                // Swap to idle task
                self.cpu.swap_context(self.task.stack_pointer_mut(), self.idle_task.stack_pointer());

                kprintln!("[MAIN_THREAD] Idle task completed/yielded");
            }
        }
    }
}
