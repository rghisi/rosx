#[cfg(not(test))]
use crate::memory::allocator::MEMORY_ALLOCATOR;
use crate::cpu::Cpu;
use crate::default_output::{KernelOutput, setup_default_output};
use crate::future::TaskCompletionFuture;
use crate::kconfig::KConfig;
use crate::kernel_services::services;
use crate::kprintln;
use crate::main_thread::MainThread;
use crate::messages::HardwareInterrupt;
use crate::state::{ExecutionContext, ExecutionState};
use crate::syscall::{task_yield, terminate_current_task};
use crate::task::TaskState::Terminated;
use crate::task::{SharedTask, Task, TaskHandle, YieldReason};
use alloc::boxed::Box;
use core::alloc::GlobalAlloc;
use core::ptr::null_mut;
use system::future::FutureHandle;
#[cfg(not(test))]
use crate::memory::allocator::{MemoryAllocator, MemoryBlocks};

pub(crate) static mut KERNEL: *mut Kernel = null_mut();

pub struct Kernel {
    kconfig: &'static KConfig,
    cpu: &'static dyn Cpu,
    main_thread: Box<MainThread>,
    execution_state: ExecutionState,
}

impl Kernel {
    pub fn new(kconfig: &'static KConfig) -> Self {
        let cpu = kconfig.cpu;
        crate::kernel_services::init();
        let main_thread = Box::new(MainThread::new());
        let main_thread_task = Task::new(0, "[K] Main Thread", main_thread_run as usize, 0);
        let main_thread_task_handle = services()
            .task_manager
            .borrow_mut()
            .add_task(main_thread_task)
            .unwrap();
        cpu.initialize_task(
            services()
                .task_manager
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
                execution_context: ExecutionContext::Kernel,
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
        let task_handle = services()
            .task_manager
            .borrow_mut()
            .add_task(idle_task)
            .unwrap();
        self.cpu.initialize_task(
            services()
                .task_manager
                .borrow_mut()
                .borrow_task_mut(task_handle)
                .unwrap(),
        );
        let _ = self.main_thread.set_idle_task(task_handle);
    }

    pub fn start(&mut self) {
        let main_thread_handle = self.execution_state.main_thread;
        let scheduler_thread_stack_pointer = services()
            .task_manager
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
        let result = services().task_manager.borrow_mut().new_task(entrypoint);
        let future_handle = match result {
            Ok(task_handle) => {
                let future = Box::new(TaskCompletionFuture::new(task_handle));
                let future_handle = services().future_registry.borrow_mut().register(future);
                self.schedule_task(task_handle);
                future_handle
            }
            Err(_) => {
                panic!("Not able to create new task");
            }
        };
        self.execution_state.preemption_enabled = prev;
        future_handle.ok_or(())
    }

    fn schedule_task(&mut self, task_handle: TaskHandle) {
        {
            let result = services().task_manager.borrow_mut().borrow_task_mut(task_handle);
            match result {
                Ok(task) => {
                    self.cpu.initialize_task(task);
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
        let task_handle = services().task_manager.borrow_mut().add_task(task).unwrap();
        self.cpu.initialize_task(
            services()
                .task_manager
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
        services().future_registry.borrow_mut().get(handle).unwrap_or(true)
    }

    pub fn task_yield(&mut self) {
        if let Some(task_handle) = self.execution_state.current_task {
            services().task_manager.borrow_mut().set_yield_reason(task_handle, YieldReason::Voluntary);
        }
        self.execution_state.switch_to_scheduler();
    }

    pub fn preempt(&mut self) {
        if self.execution_state.preemption_enabled {
            if let Some(task_handle) = self.execution_state.current_task {
                services().task_manager.borrow_mut().set_yield_reason(task_handle, YieldReason::Preempted);
            }
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
            services()
                .task_manager
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

#[cfg(not(test))]
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
    memory_blocks: &MemoryBlocks,
    default_output: &'static dyn KernelOutput,
) {
    setup_default_output(default_output);
    MemoryAllocator::print_config(memory_blocks);
    MEMORY_ALLOCATOR.init(memory_blocks);
    kprintln!("[KERNEL] Bootstrapped");
}

pub(crate) extern "C" fn task_wrapper(entry_point: usize) {
    let task_fn: fn() = unsafe { core::mem::transmute(entry_point) };

    unsafe {
        (*KERNEL).execution_state.preemption_enabled = true;
    }

    task_fn();

    terminate_current_task();
    task_yield();
}

extern "C" fn main_thread_run() -> ! {
    unsafe {
        (*KERNEL).main_thread.run();
    }

    panic!("Kernel main thread returned");
}
