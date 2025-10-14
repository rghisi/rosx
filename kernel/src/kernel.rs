use core::ptr::null_mut;
use cpu::{Cpu};
use kprintln;
use main_thread::MainThread;
use state::{CPU_PTR, CURRENT_TASK, MAIN_THREAD_PTR, MAIN_THREAD_TASK, MAIN_THREAD_TASK_PTR};
use task::{SharedTask, Task};
use wrappers::{main_thread_wrapper, task_wrapper};

pub struct Kernel {
    cpu: &'static dyn Cpu,
    main_thread: MainThread,
}

impl Kernel {
    pub fn new(
        cpu: &'static (dyn Cpu + 'static),
        idle_task: SharedTask,
    ) -> Self {
        let main_thread = MainThread::new(cpu, idle_task);

        Kernel {
            cpu,
            main_thread
        }
    }

    pub fn setup(&mut self) {
        self.cpu.setup();
        let main_thread_entrypoint = main_thread_wrapper as usize;
        unsafe {
            let mut main_thread_task = Task::new(
                0,
                "Kernel Main Thread",
                task_wrapper as usize,
                main_thread_entrypoint
            );
            MAIN_THREAD_TASK_PTR = Some(&mut *main_thread_task as *mut Task);
            MAIN_THREAD_TASK = Some(main_thread_task);
            MAIN_THREAD_PTR = Some(&mut self.main_thread as *mut MainThread);
            CPU_PTR = Some(self.cpu);
        }
    }

    pub fn schedule(&mut self, mut task: SharedTask) {
        let new_stack_pointer = self.cpu.initialize_task(
            task.stack_pointer(),
            task.entry_point(),
            task.actual_entry_point()
        );
        task.set_stack_pointer(new_stack_pointer);
        task.set_ready();
        let _ = self.main_thread.push_task(task);
    }

   pub fn initialize_task(&mut self, mut task: SharedTask) {
        let new_stack_pointer = self.cpu.initialize_task(
            task.stack_pointer(),
            task.entry_point(),
            task.actual_entry_point()
        );
        task.set_stack_pointer(new_stack_pointer);
        task.set_ready();
    }

    pub fn start(&mut self) {
        let mut task =  unsafe {
            MAIN_THREAD_TASK.take().expect("Task should be returned from yield")
        };

        let original_sp = task.stack_pointer();
        let new_stack_pointer = self.cpu.initialize_task(
            original_sp,
            task.entry_point(),
            task.actual_entry_point()
        );
        task.set_stack_pointer(new_stack_pointer);
        task.set_ready();

        unsafe {
            MAIN_THREAD_TASK = Some(task);
        }

        self.cpu.swap_context(null_mut(), new_stack_pointer);
    }
}

pub fn task_yield() {
    unsafe {
        if let Some(cpu)= CPU_PTR {
            if let Some(mut task) = CURRENT_TASK.take() {
                if let Some(main) = MAIN_THREAD_TASK_PTR {
                    let task_stack_pointer_reference = task.stack_pointer_mut();
                    CURRENT_TASK = Some(task);
                    cpu.swap_context(task_stack_pointer_reference, main.as_mut().unwrap().stack_pointer())
                }
            }
        }
    }
}