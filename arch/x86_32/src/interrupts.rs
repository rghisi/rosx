use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use kernel::kernel::kernel;
use kernel::messages::HardwareInterrupt;
use lazy_static::lazy_static;

pub static SYSTEM_TIME_MS: AtomicU32 = AtomicU32::new(0);

// ── PIC (8259A) ───────────────────────────────────────────────────────────────

const PIC_MASTER_OFFSET: u8 = 0x20;
const PIC_SLAVE_OFFSET: u8 = 0x28;
const PIC_MASTER_CMD: u16 = 0x20;
const PIC_MASTER_DATA: u16 = 0x21;
const PIC_SLAVE_CMD: u16 = 0xA0;
const PIC_SLAVE_DATA: u16 = 0xA1;
const PIC_EOI: u8 = 0x20;

// ── PIT (8253/8254) ───────────────────────────────────────────────────────────

const PIT_FREQUENCY: u32 = 1_193_182;
const TICK_RATE_HZ: u32 = 100;
const PIT_DIVISOR: u16 = (PIT_FREQUENCY / TICK_RATE_HZ) as u16;
const MS_PER_TICK: u64 = 1_000 / TICK_RATE_HZ as u64;

// ── IDT ───────────────────────────────────────────────────────────────────────

// 32-bit protected-mode IDT entry (8 bytes).
// Layout matches the IA-32 manual: two 16-bit offset halves surrounding the
// segment selector, type/attribute byte, and a reserved zero byte.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    zero: u8,
    type_attr: u8,
    offset_high: u16,
}

impl IdtEntry {
    const fn absent() -> Self {
        Self { offset_low: 0, selector: 0, zero: 0, type_attr: 0, offset_high: 0 }
    }

    fn interrupt_gate(handler: extern "x86-interrupt" fn(InterruptStackFrame)) -> Self {
        let addr = handler as *const () as usize as u32;
        Self {
            offset_low: (addr & 0xFFFF) as u16,
            selector: 0x08,  // GDT[1] — kernel code segment supplied by GRUB
            zero: 0,
            type_attr: 0x8E, // P=1, DPL=0, type=0xE (32-bit interrupt gate)
            offset_high: ((addr >> 16) & 0xFFFF) as u16,
        }
    }

    fn trap_gate(handler_addr: u32) -> Self {
        Self {
            offset_low: (handler_addr & 0xFFFF) as u16,
            selector: 0x08,
            zero: 0,
            type_attr: 0x8F, // P=1, DPL=0, type=0xF (32-bit trap gate)
            offset_high: ((handler_addr >> 16) & 0xFFFF) as u16,
        }
    }
}

// Full 256-entry IDT aligned to 16 bytes as recommended by the IA-32 manual.
#[repr(C, align(16))]
struct Idt([IdtEntry; 256]);

// 48-bit descriptor loaded by the lidt instruction.
#[repr(C, packed)]
struct IdtPtr {
    limit: u16,
    base: u32,
}

unsafe extern "C" {
    fn int80_handler();
}

lazy_static! {
    static ref IDT: Idt = {
        let mut idt = Idt([IdtEntry::absent(); 256]);
        idt.0[PIC_MASTER_OFFSET as usize]     = IdtEntry::interrupt_gate(timer_interrupt_handler);
        idt.0[PIC_MASTER_OFFSET as usize + 1] = IdtEntry::interrupt_gate(keyboard_interrupt_handler);
        idt.0[0x80] = IdtEntry::trap_gate(int80_handler as *const () as u32);
        idt
    };
}

// ── Initialisation ────────────────────────────────────────────────────────────

pub fn init() {
    let ptr = IdtPtr {
        limit: (core::mem::size_of::<Idt>() - 1) as u16,
        base: IDT.0.as_ptr() as u32,
    };
    unsafe {
        asm!("lidt [{0}]", in(reg) &ptr as *const IdtPtr as usize,
             options(nostack, readonly, preserves_flags));
    }
    init_pic();
}

fn init_pic() {
    unsafe {
        // ICW1 — begin initialisation (cascade mode, ICW4 required)
        outb(PIC_MASTER_CMD, 0x11);
        outb(PIC_SLAVE_CMD,  0x11);
        // ICW2 — interrupt vector offsets
        outb(PIC_MASTER_DATA, PIC_MASTER_OFFSET);
        outb(PIC_SLAVE_DATA,  PIC_SLAVE_OFFSET);
        // ICW3 — cascade wiring (slave on IRQ2)
        outb(PIC_MASTER_DATA, 0x04);
        outb(PIC_SLAVE_DATA,  0x02);
        // ICW4 — 8086/88 mode
        outb(PIC_MASTER_DATA, 0x01);
        outb(PIC_SLAVE_DATA,  0x01);
        // Mask all IRQs; unmask selectively via enable_timer / enable_keyboard
        outb(PIC_MASTER_DATA, 0xFF);
        outb(PIC_SLAVE_DATA,  0xFF);
    }
}

pub fn enable_timer() {
    unsafe {
        // Configure PIT channel 0: lobyte/hibyte, mode 3 (square wave), binary
        outb(0x43, 0x36);
        outb(0x40, (PIT_DIVISOR & 0xFF) as u8);
        outb(0x40, (PIT_DIVISOR >> 8) as u8);
        // Unmask IRQ0
        let mask = inb(PIC_MASTER_DATA);
        outb(PIC_MASTER_DATA, mask & !0x01);
    }
}

pub fn enable_keyboard() {
    unsafe {
        let _ = inb(0x60); // flush any pending scancode from the keyboard buffer
        // Unmask IRQ1
        let mask = inb(PIC_MASTER_DATA);
        outb(PIC_MASTER_DATA, mask & !0x02);
    }
}

// ── Interrupt stack frame (same-privilege, no error code) ─────────────────────

#[repr(C)]
pub struct InterruptStackFrame {
    eip: u32,
    cs: u32,
    eflags: u32,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

extern "x86-interrupt" fn timer_interrupt_handler(_frame: InterruptStackFrame) {
    SYSTEM_TIME_MS.fetch_add(MS_PER_TICK as u32, Relaxed);
    unsafe { outb(PIC_MASTER_CMD, PIC_EOI) };
    kernel().preempt();
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_frame: InterruptStackFrame) {
    let scancode = unsafe { inb(0x60) };
    unsafe { outb(PIC_MASTER_CMD, PIC_EOI) };
    kernel().enqueue(HardwareInterrupt::Keyboard { scancode });
}

// ── Port I/O ──────────────────────────────────────────────────────────────────

unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value,
             options(nomem, nostack, preserves_flags));
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", out("al") value, in("dx") port,
             options(nomem, nostack, preserves_flags));
    }
    value
}
