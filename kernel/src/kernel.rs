use alloc::boxed::Box;
use core::ptr::null_mut;
use cpu::{Cpu};
use kconfig::KConfig;
use kprintln;
use task_scheduler_round_robin::RoundRobin;
use state::{CPU_PTR, CURRENT_TASK, MAIN_THREAD_PTR, MAIN_THREAD_TASK};
use task::{SharedTask, Task};
use task_scheduler::TaskScheduler;
use wrappers::{scheduler_wrapper, task_wrapper};

pub struct Kernel {
    cpu: &'static dyn Cpu,
    scheduler: Box<dyn TaskScheduler>,
}

impl Kernel {

    pub fn new_kconfig(
        kconfig: &KConfig
    ) -> Self {
        Kernel {
            cpu: kconfig.cpu,
            scheduler: (kconfig.scheduler)(),
        }
    }

    pub fn setup(&mut self) {
        unsafe {
            MAIN_THREAD_PTR = Some(self.scheduler.as_mut());
            CPU_PTR = Some(self.cpu);
        }

    }

    pub fn schedule(&mut self, mut task: SharedTask) {
        self.initialize_task(task.as_mut());
        let _ = self.scheduler.push_task(task);
    }

   pub fn initialize_task(&mut self, task: &mut Task) {
       self.cpu.initialize_task_hl(task);
    }

    pub fn start(&mut self) {
        let mut scheduler_task = Task::new(
            0,
            "Kernel Main Thread",
            task_wrapper as usize,
            scheduler_wrapper as usize
        );
        self.cpu.initialize_task_hl(scheduler_task.as_mut());
        let scheduler_thread_stack_pointer = scheduler_task.as_ref().stack_pointer();

        unsafe {
            MAIN_THREAD_TASK = Some(scheduler_task);
        }


        self.cpu.swap_context(null_mut(), scheduler_thread_stack_pointer);
    }
}

pub fn task_yield() {
    unsafe {
        if let Some(cpu)= CPU_PTR {
            if let Some(mut task) = CURRENT_TASK.take() {
                if let Some(main) = &MAIN_THREAD_TASK {
                    let task_stack_pointer_reference = task.stack_pointer_mut();
                    CURRENT_TASK = Some(task);
                    cpu.swap_context(task_stack_pointer_reference, main.stack_pointer())
                }
            }
        }
    }
}