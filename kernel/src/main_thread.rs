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
    idle_task: Box<Task>,
    ready_tasks: SimpleScheduler,
    blocked_tasks: Vec<Box<Task>>,
}

impl  MainThread {

    pub(crate) fn new(cpu: &'static (dyn Cpu + 'static), idle_task: Box<Task>) -> Self {
        MainThread {
            cpu,
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
            kprintln!("[MAIN_THREAD] Loop iteration");
            // ... scheduler code
        }
    }
}
