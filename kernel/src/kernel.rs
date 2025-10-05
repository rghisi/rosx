use alloc::boxed::Box;
use cpu::{Cpu};
use main_thread::MainThread;
use scheduler::Scheduler;
use task::Task;

pub struct Kernel<'a> {
    cpu: &'a dyn Cpu,
    main_thread: MainThread,
    main_thread_task: Task,
}

impl<'a> Kernel<'a> {
    pub fn new(
        cpu: &'a (dyn Cpu + 'a),
        _: &'a mut (dyn Scheduler + 'a),
        idle_task: Box<Task>,
    ) -> Self {
        let main_thread = MainThread::new(idle_task);
        let main_thread_entrypoint = MainThread::get_vtable_entry_address(&main_thread);
        Kernel {
            cpu,
            main_thread,
            main_thread_task: Task::new(
                0,
                "Kernel Main Thread",
                main_thread_entrypoint
            )
        }
    }

    pub fn setup(&mut self) {
        self.cpu.setup();
    }

    pub fn schedule(&mut self, task: Box<Task>) {
        let mut t = self.cpu.initialize_task(task);
        t.set_ready();
        let _ = self.main_thread.push_task(t);
    }

    pub fn start(&mut self) {
        // self.cpu.swap_context(0 as *mut u8, self.main_thread_task.stack_pointer());
    }
}

