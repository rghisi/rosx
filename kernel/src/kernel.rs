use alloc::boxed::Box;
use core::ptr::{null_mut};
use cpu::Cpu;
use kconfig::KConfig;
use lazy_static::lazy_static;
use spin::Mutex;
use allocator::MEMORY_ALLOCATOR;
use core::alloc::GlobalAlloc;
use core::cell::{RefCell};
use default_output::{setup_default_output, KernelOutput};
use future::Future;
use kprintln;
use main_thread::MainThread;
use state::ExecutionState;
use syscall::{task_yield, terminate_current_task};
use system::message::{Message, MessageType};
use task::{SharedTask, Task};
use task::TaskState::Terminated;
use task_arena::{TaskHandle};
use task_manager::TaskManager;
use crate::messages::HardwareInterrupt;

pub(crate) static mut KERNEL: *mut Kernel = null_mut();

lazy_static! {
    pub (crate) static ref TASK_MANAGER: Mutex<RefCell<TaskManager>> = {
        Mutex::new(RefCell::new(TaskManager::new()))
    };
}

pub struct Kernel {
    kconfig: &'static KConfig,
    cpu: &'static dyn Cpu,
    main_thread: Box<MainThread>,
    execution_state: ExecutionState,
}

impl Kernel {

    pub fn new(
        kconfig: &'static KConfig
    ) -> Self {
        let cpu = kconfig.cpu;

        let mut main_thread = Box::new(MainThread::new());
        let ptr = main_thread.as_mut() as *const MainThread as usize;
        let main_thread_task = Task::new(
            0,
            "[K] Main Thread",
            main_thread_wrapper as usize,
            ptr
        );
        let main_thread_task_handle = TASK_MANAGER.lock().borrow_mut().add_task(main_thread_task).unwrap();
        cpu.initialize_task(main_thread_task_handle, TASK_MANAGER.lock().borrow_mut().borrow_task_mut(main_thread_task_handle).unwrap());

        Kernel {
            kconfig,
            cpu,
            main_thread,
            execution_state: ExecutionState {
                main_thread: main_thread_task_handle,
                current_task: None,
                preemption_enabled: false,
                cpu
            },
        }
    }

    pub fn setup(&mut self) {
        unsafe {
            KERNEL = self;
        }
        self.cpu.setup();
        let idle_task = (self.kconfig.idle_task_factory)();
        let task_handle = TASK_MANAGER.lock().borrow_mut().add_task(idle_task).unwrap();
        self.cpu.initialize_task(task_handle, TASK_MANAGER.lock().borrow_mut().borrow_task_mut(task_handle).unwrap());
        let _ = self.main_thread.set_idle_task(task_handle);
    }

    pub fn start(&mut self) {
        let main_thread_handle = self.execution_state.main_thread;
        let scheduler_thread_stack_pointer = TASK_MANAGER.lock().borrow().get_task_stack_pointer(main_thread_handle);
        self.cpu.enable_interrupts();
        self.execution_state.preemption_enabled = true;
        self.cpu.swap_context(null_mut(), scheduler_thread_stack_pointer);
    }

    pub fn exec(&mut self, entrypoint: usize) {
        self.execution_state.preemption_enabled = false;
        let result = TASK_MANAGER.lock().borrow_mut().new_task(entrypoint);
        match result {
            Ok(task_handle) => {
                self.schedule2(task_handle);
            }
            Err(_) => { panic!("Not able to create new task"); }
        }
        self.execution_state.preemption_enabled = true;
    }

    pub fn schedule2(&mut self, task_handle: TaskHandle) {
        {
            let m = TASK_MANAGER.lock();
            let mut mm = m.borrow_mut();
            let result = mm.borrow_task_mut(task_handle);
            match result {
                Ok(task) => {
                    self.cpu.initialize_task(task_handle, task);
                }
                Err(_) => { panic!("Not able to schedule task"); }
            }
        }
        self.main_thread.push_task(task_handle);
    }

    pub fn schedule(&mut self, mut task: SharedTask) {
        self.execution_state.preemption_enabled = false;
        let task_handle = TASK_MANAGER.lock().borrow_mut().add_task(task).unwrap();
        self.cpu.initialize_task(task_handle, TASK_MANAGER.lock().borrow_mut().borrow_task_mut(task_handle).unwrap());
        let _ = self.main_thread.push_task(task_handle);
        self.execution_state.preemption_enabled = true;
    }

    pub fn enqueue(&mut self, hardware_interrupt: HardwareInterrupt) {
        self.execution_state.preemption_enabled = false;
        self.main_thread.push_hardware_interrupt(hardware_interrupt);
        self.execution_state.preemption_enabled = true;
    }

    pub fn wait(&mut self, future: Box<dyn Future>) {
        self.execution_state.block_current_task();
        let task_handle = self.execution_state.current_task();
        self.main_thread.push_blocked(task_handle, future);
        self.execution_state.switch_to_scheduler();
    }

    pub fn task_yield(&mut self) {
        self.execution_state.switch_to_scheduler();
    }

    pub fn preempt(&mut self) {
        if self.execution_state.preemption_enabled {
            self.execution_state.switch_to_scheduler();
        }
    }

    pub fn switch_to_task(&mut self, task_handle: TaskHandle) -> TaskHandle {
        self.execution_state.switch_to_task(task_handle)
    }

    pub(crate) fn terminate_current_task(&mut self) {
        self.execution_state.preemption_enabled = false;
        if let Some(task_handle) = self.execution_state.current_task.take() {
            TASK_MANAGER.lock().borrow_mut().set_state(task_handle, Terminated);
            TASK_MANAGER.lock().borrow_mut().remove_task(task_handle);
            self.execution_state.current_task = Some(task_handle);
        }
        self.execution_state.preemption_enabled = true;
    }

    #[inline(always)]
    pub fn syscall(&self, message: &Message) -> usize {
        self.cpu.syscall(message)
    }

    #[inline(always)]
    pub fn get_system_time(&self) -> u64 {
        self.cpu.get_system_time()
    }
}

pub fn bootstrap(allocator: &'static (dyn GlobalAlloc + Sync), default_output: &'static dyn KernelOutput) {
    unsafe { MEMORY_ALLOCATOR.init(allocator); };
    setup_default_output(default_output);
    kprintln!("[KERNEL] Bootstrapped");
}

pub(crate) extern "C" fn task_wrapper(index: usize, generation: usize) {
    let task_handle = TaskHandle {
        index: index as u8,
        generation: generation as u8
    };
    let actual_entry = TASK_MANAGER.lock().borrow_mut().borrow_task(task_handle).unwrap().actual_entry_point();
    let task_fn: fn() = unsafe { core::mem::transmute(actual_entry) };
    task_fn();

    terminate_current_task();
    task_yield();
}

extern "C" fn main_thread_wrapper(index: usize, generation: usize) -> ! {
    let task_handle = TaskHandle {
        index: index as u8,
        generation: generation as u8
    };
    let main_thread_ptr = TASK_MANAGER.lock().borrow_mut().borrow_task(task_handle).unwrap().actual_entry_point();

    let main_thread = unsafe {
        let ptr_back = main_thread_ptr as *mut MainThread;
        let mt = &mut *ptr_back;
        mt
    };

    main_thread.run();

    panic!("Kernel main thread returned");
}