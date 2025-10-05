use alloc::boxed::Box;
use cpu::{Cpu};
use kprintln;
use main_thread::MainThread;
use runnable::Runnable;
use task::Task;

static mut MAIN_THREAD_PTR: Option<*mut MainThread> = None;
pub static mut MAIN_THREAD_TASK_PTR: Option<*mut Task> = None;
pub static mut CURRENT_TASK_PTR: Option<*mut Task> = None;
static mut CPU_PTR: Option<&'static dyn Cpu> = None;

pub struct Kernel {
    cpu: &'static dyn Cpu,
    main_thread: MainThread,
    main_thread_task: Box<Task>,
}

impl Kernel {
    pub fn new(
        cpu: &'static (dyn Cpu + 'static),
        idle_task: Box<Task>,
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

    pub fn schedule(&mut self, mut task: Box<Task>) {
        let new_stack_pointer = self.cpu.initialize_task(
            task.stack_pointer(),
            task.entry_point(),
            task.actual_entry_point()
        );
        task.set_stack_pointer(new_stack_pointer);
        task.set_ready();
        let _ = self.main_thread.push_task(task);
    }

   pub fn initialize_task(&mut self, mut task: Box<Task>) {
        let new_stack_pointer = self.cpu.initialize_task(
            task.stack_pointer(),
            task.entry_point(),
            task.actual_entry_point()
        );
        task.set_stack_pointer(new_stack_pointer);
        task.set_ready();
    }

    pub fn start(&mut self) {
        kprintln!("[KERNEL] start() called");

        // Check current segment selectors and RFLAGS
        let cs: u16;
        let ss: u16;
        let rflags: u64;
        unsafe {
            core::arch::asm!("mov {0:x}, cs", out(reg) cs);
            core::arch::asm!("mov {0:x}, ss", out(reg) ss);
            core::arch::asm!("pushfq", "pop {}", out(reg) rflags);
        }
        kprintln!("[KERNEL] Current CS: {:#x}", cs);
        kprintln!("[KERNEL] Current SS: {:#x}", ss);
        kprintln!("[KERNEL] Current RFLAGS: {:#x}", rflags);


        let original_sp = self.main_thread_task.stack_pointer();
        let entry = self.main_thread_task.entry_point();

        // DEBUG: Check task stack range
        let task_ptr = &*self.main_thread_task as *const _ as usize;
        kprintln!("[KERNEL] Task struct at: {:#x}", task_ptr);
        kprintln!("[KERNEL] Original SP: {:#x}", original_sp);
        kprintln!("[KERNEL] SP should be in task's own stack buffer");
        kprintln!("[KERNEL] Entry point: {:#x}", entry);

        let new_stack_pointer = self.cpu.initialize_task(
            original_sp,
            self.main_thread_task.entry_point(),
            self.main_thread_task.actual_entry_point()
        );

        kprintln!("[KERNEL] Initialized SP: {:#x}", new_stack_pointer);
        kprintln!("[KERNEL] main_thread_wrapper addr: {:#x}", main_thread_wrapper as usize);
        kprintln!("[KERNEL] MAIN_THREAD_PTR set: {}", unsafe { MAIN_THREAD_PTR.is_some() });

        // DEBUG: Try to read the first bytes at the wrapper address
        unsafe {
            let wrapper_ptr = main_thread_wrapper as *const u8;
            let first_byte = core::ptr::read_volatile(wrapper_ptr);
            kprintln!("[KERNEL] First byte at wrapper: {:#x}", first_byte);
        }

        kprintln!("[KERNEL] About to switch_to...");

        self.main_thread_task.set_stack_pointer(new_stack_pointer);
        self.main_thread_task.set_ready();

        // DEBUG: Check if memory is still intact
        let sp = self.main_thread_task.stack_pointer();
        unsafe {
            let rip_addr = (sp + 120) as *const usize;
            let rip_value = core::ptr::read_volatile(rip_addr);
            kprintln!("[KERNEL] Before switch - RIP at {:#x} = {:#x}", rip_addr as usize, rip_value);
        }

        // Use swap_context with a dummy pointer since we never return to kernel
        // The dummy pointer can be anything since we don't care about saving kernel's context
        let mut dummy_sp: usize = 0;
        self.cpu.swap_context(&mut dummy_sp as *mut usize, self.main_thread_task.stack_pointer());

        // Should never reach here
        kprintln!("[KERNEL] ERROR: Returned from kernel start!");

    }
}

extern "C" fn main_thread_wrapper() -> ! {
    kprintln!("[WRAPPER] Entered main_thread_wrapper!");

    unsafe {
        if let Some(ptr) = MAIN_THREAD_PTR {
            kprintln!("[WRAPPER] MAIN_THREAD_PTR is set, calling run()");
            let main_thread = &mut *ptr;
            main_thread.run();
        } else {
            kprintln!("[WRAPPER] ERROR: MAIN_THREAD_PTR is NULL!");
        }
    }

    kprintln!("[WRAPPER] Entering infinite loop");
    loop {}
}

/// Kernel API: Yield control back to the MainThread
/// This is called by tasks when they complete or want to yield
/// The current task's context is saved so it can be resumed later
pub extern "C" fn task_yield() {
    kprintln!("[KERNEL_API] task_yield called");

    unsafe {
        if let (Some(cpu), Some(main_task_ptr), Some(current_task_ptr)) =
            (CPU_PTR, MAIN_THREAD_TASK_PTR, CURRENT_TASK_PTR) {

            let main_task = &mut *main_task_ptr;
            let current_task = &mut *current_task_ptr;

            // Save current task's context and switch to MainThread
            cpu.swap_context(
                current_task.stack_pointer_mut(),
                main_task.stack_pointer()
            );
        }
    }

    // Should never reach here after swap_context
    loop {
        kprintln!("[KERNEL_API] Task yield should not reach here");
    }
}

