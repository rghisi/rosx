use alloc::boxed::Box;
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

        kprintln!("[KERNEL] About to initialize task:");
        kprintln!("[KERNEL]   entry_point() = {:#x}", self.main_thread_task.entry_point());
        kprintln!("[KERNEL]   actual_entry_point() = {:#x}", self.main_thread_task.actual_entry_point());

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

        // Set MAIN_THREAD_TASK_PTR so task_yield can find it
        unsafe {
            MAIN_THREAD_TASK_PTR = Some(&mut *self.main_thread_task as *mut Task);
        }

        // DEBUG: Check if memory is still intact
        let sp = self.main_thread_task.stack_pointer();
        unsafe {
            let rip_addr = (sp + 120) as *const usize;
            let cs_addr = (sp + 128) as *const usize;
            let rflags_addr = (sp + 136) as *const usize;

            let rip_value = core::ptr::read_volatile(rip_addr);
            let cs_value = core::ptr::read_volatile(cs_addr);
            let rflags_value = core::ptr::read_volatile(rflags_addr);

            kprintln!("[KERNEL] Stack frame at SP: {:#x}", sp);
            kprintln!("[KERNEL]   RIP    @ {:#x} = {:#x}", rip_addr as usize, rip_value);
            kprintln!("[KERNEL]   CS     @ {:#x} = {:#x}", cs_addr as usize, cs_value);
            kprintln!("[KERNEL]   RFLAGS @ {:#x} = {:#x}", rflags_addr as usize, rflags_value);
        }

        kprintln!("[KERNEL] About to call restore_context_and_iretq...");

        // Use swap_context with a dummy pointer since we never return to kernel
        // The dummy pointer can be anything since we don't care about saving kernel's context
        self.cpu.switch_to_kernel(new_stack_pointer);

        // Should never reach here
        kprintln!("[KERNEL] ERROR: Returned from kernel start!");

    }
}

extern "C" fn main_thread_wrapper() -> ! {
    // DEBUG: Output a character immediately via port I/O (no allocations)
    unsafe {
        core::arch::asm!(
            "mov al, 'W'",
            "out 0xe9, al",
            options(nostack)
        );
    }

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

/// Assembly yield interrupt handler calls this function
/// Called from yield_interrupt_handler_asm with the current stack pointer
/// Returns the main_thread's stack pointer
#[no_mangle]
extern "C" fn yield_handler_rust(current_sp: usize) -> usize {
    kprintln!("[YIELD_HANDLER] Called with SP: {:#x}", current_sp);

    unsafe {
        if let Some(main_task_ptr) = MAIN_THREAD_TASK_PTR {
            let main_task = &mut *main_task_ptr;

            if CURRENT_TASK.is_some() {
                // Normal case: yielding from a task back to main_thread
                let mut current_task = CURRENT_TASK.take().unwrap();
                kprintln!("[YIELD_HANDLER] Saving task {} SP to {:#x}", current_task.id(), current_sp);
                current_task.set_stack_pointer(current_sp);
                CURRENT_TASK = Some(current_task);
            } else {
                // Bootstrap case: main_thread yielding to itself
                kprintln!("[YIELD_HANDLER] Bootstrap: saving main_thread SP to {:#x}", current_sp);
                main_task.set_stack_pointer(current_sp);
            }

            // Return main_thread's stack pointer
            let new_sp = main_task.stack_pointer();
            kprintln!("[YIELD_HANDLER] Returning SP: {:#x}", new_sp);
            new_sp
        } else {
            kprintln!("[YIELD_HANDLER] ERROR: MAIN_THREAD_TASK_PTR not set!");
            current_sp // Return same SP if something is wrong
        }
    }
}

/// Assembly switch_to_task interrupt handler calls this function
/// Called from switch_to_task_interrupt_handler_asm with the current stack pointer (main_thread)
/// Returns the task's stack pointer
#[no_mangle]
extern "C" fn switch_to_task_handler_rust(current_sp: usize) -> usize {
    kprintln!("[SWITCH_HANDLER] Called with SP: {:#x}", current_sp);

    unsafe {
        if let Some(main_task_ptr) = MAIN_THREAD_TASK_PTR {
            let main_task = &mut *main_task_ptr;

            // Save main_thread's SP
            kprintln!("[SWITCH_HANDLER] Saving main_thread SP to {:#x}", current_sp);
            main_task.set_stack_pointer(current_sp);

            // Get task to switch to
            if let Some(ref task) = CURRENT_TASK {
                let task_sp = task.stack_pointer();
                kprintln!("[SWITCH_HANDLER] Switching to task {} SP: {:#x}", task.id(), task_sp);
                task_sp
            } else {
                kprintln!("[SWITCH_HANDLER] ERROR: No task to switch to!");
                current_sp
            }
        } else {
            kprintln!("[SWITCH_HANDLER] ERROR: MAIN_THREAD_TASK_PTR not set!");
            current_sp
        }
    }
}

pub fn task_yield() {
    unsafe {
        if let Some(cpu)= CPU_PTR {
            cpu.trigger_yield();
        }
    }
}