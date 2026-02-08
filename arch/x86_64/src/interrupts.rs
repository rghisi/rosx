use crate::cpu::syscall_handler_entry;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering::Relaxed;
use kernel::messages::HardwareInterrupt;
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin::Mutex;
use usrlib::println;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

const PIC_1_OFFSET: u8 = 0x20;
const PIC_2_OFFSET: u8 = 0x28;
const SYSCALL_VECTOR: u8 = 0x80;

const PIT_FREQUENCY: u32 = 1_193_182;
const TICK_RATE_HZ: u32 = 100;
const PIT_DIVISOR: u16 = (PIT_FREQUENCY / TICK_RATE_HZ) as u16;
const MS_PER_TICK: u64 = 10;

pub(crate) static SYSTEM_TIME_MS: AtomicU64 = AtomicU64::new(0);

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

static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        idt[SYSCALL_VECTOR].set_handler_fn(syscall_handler);

        unsafe {
            idt[SYSCALL_VECTOR]
                .set_handler_addr(core::mem::transmute(syscall_handler_entry as *const ()));
        }

        idt
    };
}

pub fn init() {
    IDT.load();

    unsafe {
        PICS.lock().initialize();

        use x86_64::instructions::port::Port;
        let mut pic1_data: Port<u8> = Port::new(0x21);
        let mut pic2_data: Port<u8> = Port::new(0xA1);

        pic1_data.write(0xFF);
        pic2_data.write(0xFF);
    }
}

pub fn enable_interrupts() {
    x86_64::instructions::interrupts::enable();
}

pub fn enable_keyboard() {
    use x86_64::instructions::port::Port;
    unsafe {
        let mut port: Port<u8> = Port::new(0x60);
        let _: u8 = port.read();

        let mut pic1_data: Port<u8> = Port::new(0x21);
        let current_mask = pic1_data.read();
        pic1_data.write(current_mask & !0x02);
    }
}

pub fn enable_timer() {
    unsafe {
        set_frequency_to_100hz();
        let mut pic1_data: Port<u8> = Port::new(0x21);
        let current_mask = pic1_data.read();
        pic1_data.write(current_mask & !0x01);
    }
}

fn set_frequency_to_100hz() {
    unsafe {
        let mut command: Port<u8> = Port::new(0x43);
        let mut data: Port<u8> = Port::new(0x40);

        command.write(0x36);

        data.write((PIT_DIVISOR & 0xFF) as u8);
        data.write((PIT_DIVISOR >> 8) as u8);
    }
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    SYSTEM_TIME_MS.fetch_add(MS_PER_TICK, Relaxed);

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }

    kernel::syscall::preempt();
}

const KEYBOARD_PORT: u16 = 0x60;
extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let mut port: Port<u8> = Port::new(KEYBOARD_PORT);
    let scancode: u8 = unsafe { port.read() };

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    };

    let keyboard_interrupt = HardwareInterrupt::Keyboard { scancode };
    kernel::syscall::enqueue_hardware_interrupt(keyboard_interrupt);
}

extern "x86-interrupt" fn syscall_handler(_stack_frame: InterruptStackFrame) {
    println!("syscall handler called!");
}
