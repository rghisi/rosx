use alloc::boxed::Box;
use core::ptr::null_mut;
use cpu::{Cpu};
use kprintln;
use main_thread::MainThread;
use runnable::Runnable;
use task::{SharedTask, Task};

static mut MAIN_THREAD_PTR: Option<*mut MainThread> = None;
pub static mut MAIN_THREAD_TASK_PTR: Option<*mut Task> = None;
pub static mut CURRENT_TASK: Option<SharedTask> = None;
static mut CPU_PTR: Option<&'static dyn Cpu> = None;

pub struct Kernel {
    cpu: &'static dyn Cpu,
    main_thread: MainThread,
    main_thread_task: SharedTask,
}

impl Kernel {
    pub fn new(
        cpu: &'static (dyn Cpu + 'static),
        idle_task: SharedTask,
    ) -> Self {
        let main_thread = MainThread::new(cpu, idle_task);
        let main_thread_entrypoint = main_thread_wrapper as usize;

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
        unsafe {
            MAIN_THREAD_PTR = Some(&mut self.main_thread as *mut MainThread);
            CPU_PTR = Some(self.cpu);
            // MAIN_THREAD_TASK_PTR and CURRENT_TASK_PTR will be set later
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
        let original_sp = self.main_thread_task.stack_pointer();

        let new_stack_pointer = self.cpu.initialize_task(
            original_sp,
            self.main_thread_task.entry_point(),
            self.main_thread_task.actual_entry_point()
        );

        self.main_thread_task.set_stack_pointer(new_stack_pointer);
        self.main_thread_task.set_ready();

        unsafe {
            MAIN_THREAD_TASK_PTR = Some(&mut *self.main_thread_task as *mut Task);
        }

        self.cpu.swap_context(null_mut(), new_stack_pointer);
    }
}

extern "C" fn main_thread_wrapper() -> ! {
    unsafe {
        if let Some(ptr) = MAIN_THREAD_PTR {
            let main_thread = &mut *ptr;
            main_thread.run();
        }
    }

    loop {}
}

/// Assembly yield interrupt handler calls this function
/// Called from yield_interrupt_handler_asm with the current stack pointer
/// Returns the main_thread's stack pointer
#[no_mangle]
extern "C" fn yield_handler_rust(current_sp: usize) -> usize {
    unsafe {
        if let Some(main_task_ptr) = MAIN_THREAD_TASK_PTR {
            let main_task = &mut *main_task_ptr;

            if CURRENT_TASK.is_some() {
                let mut current_task = CURRENT_TASK.take().unwrap();
                current_task.set_stack_pointer(current_sp);
                CURRENT_TASK = Some(current_task);
            } else {
                main_task.set_stack_pointer(current_sp);
            }

            main_task.stack_pointer()
        } else {
            current_sp
        }
    }
}

/// Assembly switch_to_task interrupt handler calls this function
/// Called from switch_to_task_interrupt_handler_asm with the current stack pointer (main_thread)
/// Returns the task's stack pointer
#[no_mangle]
extern "C" fn switch_to_task_handler_rust(current_sp: usize) -> usize {
    unsafe {
        if let Some(main_task_ptr) = MAIN_THREAD_TASK_PTR {
            let main_task = &mut *main_task_ptr;
            main_task.set_stack_pointer(current_sp);

            if let Some(ref task) = CURRENT_TASK {
                task.stack_pointer()
            } else {
                current_sp
            }
        } else {
            current_sp
        }
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