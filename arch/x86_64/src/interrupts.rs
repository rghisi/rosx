use crate::cpu::syscall_handler_entry;
use crate::interrupt_controller::PicController;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering::Relaxed;
use kernel::once::Once;
use kernel::platform::interrupt::{self, InterruptController};
use kernel::kernel::kernel;
use usrlib::println;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

const SYSCALL_VECTOR: u8 = 0x80;

const PIT_FREQUENCY: u32 = 1_193_182;
const TICK_RATE_HZ: u32 = 100;
const PIT_DIVISOR: u16 = (PIT_FREQUENCY / TICK_RATE_HZ) as u16;
const MS_PER_TICK: u64 = 10;

pub(crate) static SYSTEM_TIME_MS: AtomicU64 = AtomicU64::new(0);
static USING_LOCAL_APIC: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum InterruptIndex {
    Timer = PicController::VECTOR_BASE,
    Keyboard = PicController::VECTOR_BASE + 1,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }

    fn as_irq(self) -> u32 {
        (self.as_u8() - PicController::VECTOR_BASE) as u32
    }
}

static IDT: Once<InterruptDescriptorTable> = Once::new();

fn build_idt() -> InterruptDescriptorTable {
    let mut idt = InterruptDescriptorTable::new();
    idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
    idt[SYSCALL_VECTOR].set_handler_fn(syscall_handler);

    unsafe {
        idt[SYSCALL_VECTOR]
            .set_handler_addr(core::mem::transmute(syscall_handler_entry as *const ()));
    }

    idt
}

pub fn init() {
    IDT.get_or_init(build_idt).load();
    if crate::apic::cpu_supports_local_apic() {
        kernel::kprintln!("[KERNEL] Interrupt controller: Local APIC + I/O APIC");
        USING_LOCAL_APIC.store(true, Relaxed);
        interrupt::install::<crate::apic::LocalApic>();
    } else {
        kernel::kprintln!("[KERNEL] Interrupt controller: 8259 PIC");
        interrupt::install::<PicController>();
    }
    let _ = interrupt::register_handler(InterruptIndex::Timer.as_irq(), timer_handler);
    let _ = interrupt::register_handler(InterruptIndex::Keyboard.as_irq(), keyboard_handler);
}

pub fn enable_keyboard() {
    unsafe {
        let mut port: Port<u8> = Port::new(0x60);
        let _: u8 = port.read();
    }
    let _ = interrupt::unmask(InterruptIndex::Keyboard.as_irq());
}

pub fn enable_timer() {
    if USING_LOCAL_APIC.load(Relaxed) {
        crate::apic::arm_periodic_tick(MS_PER_TICK as u32);
    } else {
        set_frequency_to_100hz();
        let _ = interrupt::unmask(InterruptIndex::Timer.as_irq());
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
    interrupt::dispatch_irq(InterruptIndex::Timer.as_irq());
    kernel().apply_pending_preemption();
}

const KEYBOARD_PORT: u16 = 0x60;

fn timer_handler(_irq: u32) {
    SYSTEM_TIME_MS.fetch_add(MS_PER_TICK, Relaxed);
    kernel().preempt();
}

fn keyboard_handler(_irq: u32) {
    let mut port: Port<u8> = Port::new(KEYBOARD_PORT);
    let scancode: u8 = unsafe { port.read() };
    let frame = system::ipc::Message::new(
        system::ipc::KERNEL_EVENT_CONN,
        &input_protocol::wire::encode_scancode(scancode),
    );
    kernel::ipc::deliver_to_service(input_protocol::wire::SOURCE_NAME_PS2KEYBOARD, frame);
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    interrupt::dispatch_irq(InterruptIndex::Keyboard.as_irq());
}

extern "x86-interrupt" fn syscall_handler(_stack_frame: InterruptStackFrame) {
    println!("syscall handler called!");
}
