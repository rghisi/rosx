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
        // unsafe {
        //     unsafe extern "C" {
        //         fn yield_interrupt_handler_asm();
        //         fn switch_to_task_interrupt_handler_asm();
        //     }
        //
        //     // Transmute the raw function pointers to the expected type
        //     // This is safe because our assembly handlers follow the x86-64 interrupt ABI
        //     let yield_handler: extern "x86-interrupt" fn(InterruptStackFrame) =
        //         core::mem::transmute(yield_interrupt_handler_asm as *const ());
        //     let switch_handler: extern "x86-interrupt" fn(InterruptStackFrame) =
        //         core::mem::transmute(switch_to_task_interrupt_handler_asm as *const ());
        //
        //     idt[YIELD_INTERRUPT_VECTOR].set_handler_fn(yield_handler);
        //     idt[SWITCH_TO_TASK_INTERRUPT_VECTOR].set_handler_fn(switch_handler);
        // }

        idt
    };
}

/// Initialize interrupts (IDT and PIC setup, but don't enable yet)
pub fn init() {
    IDT.load();

    unsafe {
        PICS.lock().initialize();
    }

    unsafe {
        use x86_64::instructions::port::Port;
        let mut pic1_data: Port<u8> = Port::new(0x21);
        let mut pic2_data: Port<u8> = Port::new(0xA1);

        pic1_data.write(0xFF);
        pic2_data.write(0xFF);
    }
}

/// Enable hardware interrupts
pub fn enable_interrupts() {
    use x86_64::instructions::port::Port;
    unsafe {
        let mut port: Port<u8> = Port::new(0x60);
        let _: u8 = port.read();
    }

    x86_64::instructions::interrupts::enable();

    unsafe {
        let mut pic1_data: Port<u8> = Port::new(0x21);
        let current_mask = pic1_data.read();
        pic1_data.write(current_mask & !0x02);
    }
}

/// Enable timer interrupt for preemptive scheduling
pub fn enable_timer() {
    unsafe {
        use x86_64::instructions::port::Port;
        let mut pic1_data: Port<u8> = Port::new(0x21);
        let current_mask = pic1_data.read();
        pic1_data.write(current_mask & !0x01);
    }
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
    use x86_64::instructions::port::Port;
    let mut port: Port<u8> = Port::new(0x60);
    let _scancode: u8 = unsafe { port.read() };

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

// Yield interrupt handler is now implemented purely in assembly
// See yield_interrupt_handler_asm in context_switching.S
// It calls yield_handler_rust() in kernel.rs for scheduling logic
