use alloc::boxed::Box;
use alloc::vec::Vec;
use cpu::Cpu;
use kprintln;
use runnable::Runnable;
use scheduler::Scheduler;
use simple_scheduler::SimpleScheduler;
use task::{SharedTask, Task, TaskEntryPoint};
use task::TaskState::{Blocked, Created, Ready, Running};

pub(crate) struct MainThread {
    cpu: &'static dyn Cpu,
    task: SharedTask,
    idle_task: SharedTask,
    ready_tasks: SimpleScheduler,
    blocked_tasks: Vec<SharedTask>,
}

impl MainThread {

    pub(crate) fn new(cpu: &'static (dyn Cpu + 'static), mut idle_task: SharedTask) -> Self {
        let main_task = Task::new(0, "Main Thread", 0);

        let new_stack_pointer = cpu.initialize_task(
            idle_task.stack_pointer(),
            idle_task.entry_point(),
            idle_task.actual_entry_point()
        );
        idle_task.set_stack_pointer(new_stack_pointer);
        idle_task.set_ready();

        MainThread {
            cpu,
            task: main_task,
            idle_task,
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
        kprintln!("[MAIN_THREAD] run() called!");

        // Set up global pointers for task_yield API
        unsafe {
            crate::kernel::MAIN_THREAD_TASK_PTR = Some(&mut *self.task as *mut Task);
        }

        // Now that task system is ready, enable interrupts
        kprintln!("[MAIN_THREAD] Task system ready, enabling interrupts");
        self.cpu.enable_interrupts();

        // CRITICAL: Bootstrap into interrupt-driven mode
        // This yield will save our context as an interrupt frame and return via iretq
        // After this, all context switches will be interrupt-driven
        kprintln!("[MAIN_THREAD] Bootstrapping into interrupt mode...");
        crate::kernel::task_yield();
        kprintln!("[MAIN_THREAD] Now running in interrupt-driven mode!");

        loop {
            kprintln!("[MAIN_THREAD] Loop start");
            if let Some(mut task) = self.ready_tasks.take_next() {
                kprintln!("[MAIN_THREAD] Scheduling task: {}", task.id());
                task.set_running();

                // Set the task as CURRENT_TASK so the interrupt handler can find it
                unsafe {
                    crate::kernel::CURRENT_TASK = Some(task);
                }

                // Trigger INT 0x31 to switch to the task
                // This will save our context and restore the task's context
                self.cpu.trigger_switch_to_task();

                let mut task = unsafe {
                    crate::kernel::CURRENT_TASK.take().expect("Task should be returned from yield")
                };

                kprintln!("[MAIN_THREAD] Returned from task: {}", task.id());

                if task.state() == Running {
                    kprintln!("[MAIN_THREAD] Task yielded, re-queuing");
                    task.set_ready();
                    let _ = self.ready_tasks.offer(task);
                } else {
                    // Task completed (wrapper set it to terminated)
                    kprintln!("[MAIN_THREAD] Task terminated: {}", task.state());
                }
            } else {
                panic!("[MAIN_THREAD] Finito!");
                // kprintln!("[MAIN_THREAD] No ready tasks, running idle task");
                //
                // let idle_sp = self.idle_task.stack_pointer();
                //
                // // Transfer ownership to CURRENT_TASK for task_yield
                // unsafe {
                //     crate::kernel::CURRENT_TASK = Some(core::mem::replace(
                //         &mut self.idle_task,
                //         Task::new(0, "placeholder", 0) // Temporary placeholder
                //     ));
                // }
                //
                // // Swap to idle task
                // self.cpu.swap_context(self.task.stack_pointer_mut(), idle_sp);
                //
                // // Take back idle task
                // let returned_idle = unsafe {
                //     crate::kernel::CURRENT_TASK.take().expect("Idle task should be returned")
                // };
                // self.idle_task = returned_idle;
                //
                // kprintln!("[MAIN_THREAD] Idle task completed/yielded");
            }
            kprintln!("[MAIN_THREAD] Loop end");
        }
    }
}
