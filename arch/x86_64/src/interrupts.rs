use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use pic8259::ChainedPics;
use spin::Mutex;
use lazy_static::lazy_static;
use kernel::kprintln;

/// PIC (Programmable Interrupt Controller) configuration
/// Offset PIC interrupts to 0x20-0x2F to avoid conflicts with CPU exceptions (0x00-0x1F)
pub const PIC_1_OFFSET: u8 = 0x20;
pub const PIC_2_OFFSET: u8 = 0x28;

/// Hardware interrupt numbers (after remapping)
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard = PIC_1_OFFSET + 1,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Chained PICs (primary and secondary)
pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

lazy_static! {
    /// Static IDT
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        idt
    };
}

/// Initialize interrupts (IDT and PIC setup, but don't enable yet)
pub fn init() {
    kprintln!("[INTERRUPTS] Initializing IDT and PIC");

    // Load the IDT
    IDT.load();

    kprintln!("[INTERRUPTS] IDT loaded");

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

/// Keyboard interrupt handler
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

    // Call task_yield to switch to another task
    kprintln!("[INTERRUPT] Calling task_yield");
    kernel::kernel::task_yield();
    kprintln!("[INTERRUPT] Returned from task_yield, handler ending");
}
