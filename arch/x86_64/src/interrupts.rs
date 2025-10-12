use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use pic8259::ChainedPics;
use spin::Mutex;
use lazy_static::lazy_static;
use kernel::kprintln;

/// PIC (Programmable Interrupt Controller) configuration
/// Offset PIC interrupts to 0x20-0x2F to avoid conflicts with CPU exceptions (0x00-0x1F)
const PIC_1_OFFSET: u8 = 0x20;
const PIC_2_OFFSET: u8 = 0x28;

/// Software interrupt vectors for context switching
/// These are user-defined interrupts triggered by software (INT instruction)
const YIELD_INTERRUPT_VECTOR: u8 = 0x30;           // Task yields back to kernel
const SWITCH_TO_TASK_INTERRUPT_VECTOR: u8 = 0x31;  // Kernel switches to task

/// Hardware interrupt numbers (after remapping)
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard = PIC_1_OFFSET + 1,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Chained PICs (primary and secondary)
static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

lazy_static! {
    /// Static IDT
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();

        // Hardware interrupts
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);

        // Software interrupts for context switching
        // Use the raw assembly handlers
        unsafe {
            unsafe extern "C" {
                fn yield_interrupt_handler_asm();
                fn switch_to_task_interrupt_handler_asm();
            }

            // Transmute the raw function pointers to the expected type
            // This is safe because our assembly handlers follow the x86-64 interrupt ABI
            let yield_handler: extern "x86-interrupt" fn(InterruptStackFrame) =
                core::mem::transmute(yield_interrupt_handler_asm as *const ());
            let switch_handler: extern "x86-interrupt" fn(InterruptStackFrame) =
                core::mem::transmute(switch_to_task_interrupt_handler_asm as *const ());

            idt[YIELD_INTERRUPT_VECTOR].set_handler_fn(yield_handler);
            idt[SWITCH_TO_TASK_INTERRUPT_VECTOR].set_handler_fn(switch_handler);
        }

        idt
    };
}

/// Initialize interrupts (IDT and PIC setup, but don't enable yet)
pub fn init() {
    kprintln!("[INTERRUPTS] Initializing IDT and PIC");

    // Load the IDT
    IDT.load();

    kprintln!("[INTERRUPTS] IDT loaded");
    kprintln!("[INTERRUPTS] - Timer interrupt registered at vector 0x{:02x}", InterruptIndex::Timer.as_u8());
    kprintln!("[INTERRUPTS] - Keyboard interrupt registered at vector 0x{:02x}", InterruptIndex::Keyboard.as_u8());
    kprintln!("[INTERRUPTS] - Yield interrupt registered at vector 0x{:02x}", YIELD_INTERRUPT_VECTOR);
    kprintln!("[INTERRUPTS] - Switch-to-task interrupt registered at vector 0x{:02x}", SWITCH_TO_TASK_INTERRUPT_VECTOR);

    // Initialize and remap PICs
    unsafe {
        PICS.lock().initialize();
    }

    kprintln!("[INTERRUPTS] PICs initialized");

    // Mask all interrupts in the PIC initially
    unsafe {
        use x86_64::instructions::port::Port;
        let mut pic1_data: Port<u8> = Port::new(0x21);
        let mut pic2_data: Port<u8> = Port::new(0xA1);

        // Mask all interrupts (0xFF = all masked)
        pic1_data.write(0xFF);
        pic2_data.write(0xFF);
    }

    kprintln!("[INTERRUPTS] All PIC interrupts masked");
    kprintln!("[INTERRUPTS] Setup complete (interrupts still disabled)");
}

/// Enable hardware interrupts
pub fn enable_interrupts() {
    kprintln!("[INTERRUPTS] Enabling interrupts");

    // Clear any pending keyboard data
    use x86_64::instructions::port::Port;
    unsafe {
        let mut port: Port<u8> = Port::new(0x60);
        let _: u8 = port.read();  // Discard any pending scancode
    }

    kprintln!("[INTERRUPTS] Cleared pending keyboard data");

    // Enable CPU interrupts first (while PIC interrupts are still masked)
    x86_64::instructions::interrupts::enable();

    kprintln!("[INTERRUPTS] CPU interrupts enabled");

    // Now unmask the keyboard interrupt in the PIC
    unsafe {
        let mut pic1_data: Port<u8> = Port::new(0x21);
        let current_mask = pic1_data.read();
        // Unmask IRQ1 (keyboard) - bit 1
        pic1_data.write(current_mask & !0x02);
    }

    kprintln!("[INTERRUPTS] Keyboard interrupt unmasked and ready");
}

/// Enable timer interrupt for preemptive scheduling
pub fn enable_timer() {
    kprintln!("[INTERRUPTS] Enabling timer interrupt");

    unsafe {
        use x86_64::instructions::port::Port;
        let mut pic1_data: Port<u8> = Port::new(0x21);
        let current_mask = pic1_data.read();
        // Unmask IRQ0 (timer) - bit 0
        pic1_data.write(current_mask & !0x01);
    }

    kprintln!("[INTERRUPTS] Timer interrupt enabled (will tick at ~18.2 Hz)");
}

// ============================================================================
// Interrupt Handlers
// ============================================================================

/// Timer interrupt handler (IRQ0)
/// This triggers preemptive multitasking by forcing a yield
extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // Send EOI (End of Interrupt) to PIC first to allow nested interrupts
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }

    // Trigger yield interrupt (INT 0x30) to perform preemptive context switch
    // This will save the current task's context and switch back to main_thread
    unsafe {
        core::arch::asm!("int 0x30");
    }
}

/// Keyboard interrupt handler (IRQ1)
extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    kprintln!("[INTERRUPT] Keyboard interrupt!");

    // Read scancode from keyboard data port
    use x86_64::instructions::port::Port;
    let mut port: Port<u8> = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };

    kprintln!("[INTERRUPT] Scancode: {:#x}", scancode);

    // Send EOI (End of Interrupt) to PIC
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

// Yield interrupt handler is now implemented purely in assembly
// See yield_interrupt_handler_asm in context_switching.S
// It calls yield_handler_rust() in kernel.rs for scheduling logic
