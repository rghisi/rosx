#[cfg(not(test))]
use crate::allocator::{HEAP_ALLOCATOR, MEMORY_ALLOCATOR, initialize_heap};
use crate::cpu::Cpu;
use crate::default_output::{KernelOutput, setup_default_output};
use crate::future::{FutureRegistry, TaskCompletionFuture};
use crate::kconfig::KConfig;
use crate::kprintln;
use crate::main_thread::MainThread;
use crate::messages::HardwareInterrupt;
use crate::state::ExecutionState;
use crate::syscall::{task_yield, terminate_current_task};
use crate::task::TaskState::Terminated;
use crate::task::{SharedTask, Task, TaskHandle};
use crate::task_manager::TaskManager;
use alloc::boxed::Box;
use core::alloc::GlobalAlloc;
use core::cell::RefCell;
use core::ptr::null_mut;
use lazy_static::lazy_static;
use spin::Mutex;
use system::future::FutureHandle;
use system::memory::MemoryRegion;

pub(crate) static mut KERNEL: *mut Kernel = null_mut();

lazy_static! {
    pub(crate) static ref TASK_MANAGER: Mutex<RefCell<TaskManager>> =
        Mutex::new(RefCell::new(TaskManager::new()));
    pub(crate) static ref FUTURE_REGISTRY: FutureRegistry = FutureRegistry::new();
}

pub struct Kernel {
    kconfig: &'static KConfig,
    cpu: &'static dyn Cpu,
    main_thread: Box<MainThread>,
    execution_state: ExecutionState,
}

impl Kernel {
    pub fn new(kconfig: &'static KConfig) -> Self {
        let cpu = kconfig.cpu;

        let mut main_thread = Box::new(MainThread::new());
        let ptr = main_thread.as_mut() as *const MainThread as usize;
        let main_thread_task = Task::new(0, "[K] Main Thread", main_thread_wrapper as usize, ptr);
        let main_thread_task_handle = TASK_MANAGER
            .lock()
            .borrow_mut()
            .add_task(main_thread_task)
            .unwrap();
        cpu.initialize_task(
            main_thread_task_handle,
            TASK_MANAGER
                .lock()
                .borrow_mut()
                .borrow_task_mut(main_thread_task_handle)
                .unwrap(),
        );

        Kernel {
            kconfig,
            cpu,
            main_thread,
            execution_state: ExecutionState {
                main_thread: main_thread_task_handle,
                current_task: None,
                preemption_enabled: false,
                kernel_mode: true,
                cpu,
            },
        }
    }

    pub fn setup(&mut self) {
        unsafe {
            KERNEL = self;
        }
        self.cpu.setup();
        let idle_task = (self.kconfig.idle_task_factory)();
        let task_handle = TASK_MANAGER
            .lock()
            .borrow_mut()
            .add_task(idle_task)
            .unwrap();
        self.cpu.initialize_task(
            task_handle,
            TASK_MANAGER
                .lock()
                .borrow_mut()
                .borrow_task_mut(task_handle)
                .unwrap(),
        );
        let _ = self.main_thread.set_idle_task(task_handle);
    }

    pub fn start(&mut self) {
        let main_thread_handle = self.execution_state.main_thread;
        let scheduler_thread_stack_pointer = TASK_MANAGER
            .lock()
            .borrow()
            .get_task_stack_pointer(main_thread_handle);
        self.cpu.enable_interrupts();
        self.execution_state.preemption_enabled = false;
        self.cpu
            .swap_context(null_mut(), scheduler_thread_stack_pointer);
    }

    pub fn exec(&mut self, entrypoint: usize) -> Result<FutureHandle, ()> {
        let prev = self.execution_state.preemption_enabled;
        self.execution_state.preemption_enabled = false;
        let result = TASK_MANAGER.lock().borrow_mut().new_task(entrypoint);
        let future_handle = match result {
            Ok(task_handle) => {
                let future = Box::new(TaskCompletionFuture::new(task_handle));
                let future_handle = FUTURE_REGISTRY.register(future);
                self.schedule2(task_handle);
                future_handle
            }
            Err(_) => {
                panic!("Not able to create new task");
            }
        };
        self.execution_state.preemption_enabled = prev;
        future_handle.ok_or(())
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
                Err(_) => {
                    panic!("Not able to schedule task");
                }
            }
        }
        self.main_thread.push_task(task_handle);
    }

    pub fn schedule(&mut self, task: SharedTask) {
        let prev = self.execution_state.preemption_enabled;
        self.execution_state.preemption_enabled = false;
        let task_handle = TASK_MANAGER.lock().borrow_mut().add_task(task).unwrap();
        self.cpu.initialize_task(
            task_handle,
            TASK_MANAGER
                .lock()
                .borrow_mut()
                .borrow_task_mut(task_handle)
                .unwrap(),
        );
        self.main_thread.push_task(task_handle);
        self.execution_state.preemption_enabled = prev;
    }

    pub fn enqueue(&mut self, hardware_interrupt: HardwareInterrupt) {
        let prev = self.execution_state.preemption_enabled;
        self.execution_state.preemption_enabled = false;
        self.main_thread.push_hardware_interrupt(hardware_interrupt);
        self.execution_state.preemption_enabled = prev;
    }

    pub fn wait_future(&mut self, handle: FutureHandle) {
        self.execution_state.block_current_task();
        let task_handle = self.execution_state.current_task();
        self.main_thread.push_blocked(task_handle, handle);
        self.execution_state.switch_to_scheduler();
    }

    pub fn is_future_completed(&self, handle: FutureHandle) -> bool {
        FUTURE_REGISTRY.get(handle).unwrap_or(true)
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
        let prev = self.execution_state.preemption_enabled;
        self.execution_state.preemption_enabled = false;
        if let Some(task_handle) = self.execution_state.current_task.take() {
            TASK_MANAGER
                .lock()
                .borrow_mut()
                .set_state(task_handle, Terminated);
            self.execution_state.current_task = Some(task_handle);
        }
        self.execution_state.preemption_enabled = prev;
    }

    #[inline(always)]
    pub fn get_system_time(&self) -> u64 {
        self.cpu.get_system_time()
    }
}

unsafe impl GlobalAlloc for Kernel {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let interrupts_enabled = self.cpu.are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.disable_interrupts();
        }

        let ptr = unsafe { MEMORY_ALLOCATOR.alloc(layout) };

        if interrupts_enabled {
            self.cpu.enable_interrupts();
            // Give a chance for any pending interrupt to fire
            unsafe { core::arch::asm!("nop"); }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        let interrupts_enabled = self.cpu.are_interrupts_enabled();
        if interrupts_enabled {
            self.cpu.disable_interrupts();
        }

        unsafe { MEMORY_ALLOCATOR.dealloc(ptr, layout) };

        if interrupts_enabled {
            self.cpu.enable_interrupts();
            // Give a chance for any pending interrupt to fire
            unsafe { core::arch::asm!("nop"); }
        }
    }
}

#[cfg(not(test))]
pub fn bootstrap(
    regions: &[MemoryRegion],
    default_output: &'static dyn KernelOutput,
) {
    unsafe {
        MEMORY_ALLOCATOR.init(&HEAP_ALLOCATOR);
    };
    setup_default_output(default_output);
    initialize_heap(regions);
    kprintln!("[KERNEL] Bootstrapped");
}

pub(crate) extern "C" fn task_wrapper(index: usize, generation: usize) {
    let task_handle = TaskHandle {
        index: index as u8,
        generation: generation as u8,
    };
    let actual_entry = TASK_MANAGER
        .lock()
        .borrow_mut()
        .borrow_task(task_handle)
        .unwrap()
        .actual_entry_point();
    let task_fn: fn() = unsafe { core::mem::transmute(actual_entry) };

    unsafe {
        (*KERNEL).execution_state.preemption_enabled = true;
    }

    task_fn();

    terminate_current_task();
    task_yield();
}

extern "C" fn main_thread_wrapper(index: usize, generation: usize) -> ! {
    let task_handle = TaskHandle {
        index: index as u8,
        generation: generation as u8,
    };
    let main_thread_ptr = TASK_MANAGER
        .lock()
        .borrow_mut()
        .borrow_task(task_handle)
        .unwrap()
        .actual_entry_point();

    let main_thread = unsafe {
        let ptr_back = main_thread_ptr as *mut MainThread;

        &mut *ptr_back
    };

    main_thread.run();

    panic!("Kernel main thread returned");
}
