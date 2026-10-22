use kernel::interrupt::{InterruptController, InterruptError, Polarity, Result, TriggerMode};
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

const LOCAL_APIC_MMIO_BASE: u64 = 0xFEE0_0000;
const IO_APIC_MMIO_BASE: u64 = 0xFEC0_0000;

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const APIC_BASE_GLOBAL_ENABLE: u64 = 1 << 11;
const APIC_BASE_DISABLE_EXEC: u64 = 1 << 10;

const LAPIC_ID: u64 = 0x020;
const LAPIC_SPIV: u64 = 0x0F0;
const LAPIC_EOI: u64 = 0x0B0;
const LAPIC_INIT_COUNT: u64 = 0x380;
const LAPIC_CURRENT_COUNT: u64 = 0x390;
const LAPIC_TDCR: u64 = 0x3E0;
const LAPIC_LVT_TIMER: u64 = 0x320;
const LAPIC_LINT0: u64 = 0x350;
const LAPIC_LINT1: u64 = 0x360;
const LAPIC_LVT_ERROR: u64 = 0x370;

const SPIV_ENABLE: u32 = 1 << 8;
const LVT_MASKED: u32 = 1 << 16;
const SPURIOUS_VECTOR: u32 = 0xFF;

const IO_APIC_VERSION_SELECT: u32 = 0x01;
const IO_APIC_INDEX_PORT: usize = 0;
const IO_APIC_WINDOW_PORT: usize = 4;
const IO_APIC_REDIRECTION_BASE: u32 = 0x10;

const IO_APIC_RTE_POLARITY_LOW: u64 = 1 << 13;
const IO_APIC_RTE_LEVEL: u64 = 1 << 15;
const IO_APIC_RTE_MASKED: u64 = 1 << 16;

const PIC1_IMR_PORT: u16 = 0x21;
const PIC2_IMR_PORT: u16 = 0xA1;

const TIMER_VECTOR: u32 = 0x20;
const TIMER_MODE_PERIODIC: u32 = 1 << 17;

const PIT_COMMAND_PORT: u16 = 0x43;
const PIT_CHANNEL2_PORT: u16 = 0x42;
const PIT_TIMER_CTRL_PORT: u16 = 0x61;
const PIT_OUT2_MASK: u8 = 1 << 5;
const PIT_LATCH_ONE_MILLISECOND: u16 = 1193;

static PHYS_MEM_OFFSET: AtomicU64 = AtomicU64::new(0);

pub fn set_phys_offset(offset: u64) {
    PHYS_MEM_OFFSET.store(offset, Ordering::Relaxed);
}

fn phys_offset() -> u64 {
    PHYS_MEM_OFFSET.load(Ordering::Relaxed)
}

pub fn cpu_supports_local_apic() -> bool {
    let edx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {0:e}, ebx",
            "pop rbx",
            out(reg) _,
            inlateout("eax") 1u32 => _,
            lateout("ecx") _,
            lateout("edx") edx,
        );
    }
    edx & (1 << 9) != 0
}

pub struct LocalApic;

impl InterruptController for LocalApic {
    const MAX_IRQ: u32 = 15;
    const VECTOR_BASE: u8 = 0x20;

    fn new() -> Self {
        disable_legacy_pic();
        enable_local_apic();
        lapic_write(LAPIC_SPIV, SPIV_ENABLE | SPURIOUS_VECTOR);
        lapic_write(LAPIC_LVT_TIMER, LVT_MASKED);
        lapic_write(LAPIC_LINT0, LVT_MASKED);
        lapic_write(LAPIC_LINT1, LVT_MASKED);
        lapic_write(LAPIC_LVT_ERROR, LVT_MASKED | SPURIOUS_VECTOR);
        program_ioapic_redirections();
        LocalApic
    }

    fn configure(
        &mut self,
        irq: u32,
        vector: u8,
        trigger: TriggerMode,
        polarity: Polarity,
    ) -> Result {
        if irq > Self::MAX_IRQ {
            return Err(InterruptError::InvalidIrq);
        }
        if irq == 0 {
            return Err(InterruptError::Unsupported);
        }
        let mut entry = vector as u64 | IO_APIC_RTE_MASKED | destination_field();
        if trigger == TriggerMode::Level {
            entry |= IO_APIC_RTE_LEVEL;
        }
        if polarity == Polarity::Low {
            entry |= IO_APIC_RTE_POLARITY_LOW;
        }
        ioapic_write_entry(irq, entry);
        Ok(())
    }

    fn mask(&mut self, irq: u32) -> Result {
        update_source_mask(irq, true)
    }

    fn unmask(&mut self, irq: u32) -> Result {
        update_source_mask(irq, false)
    }

    fn eoi(&mut self, _irq: u32) {
        lapic_write(LAPIC_EOI, 0);
    }
}

pub fn arm_periodic_tick(millis: u32) {
    let ticks = calibrate_ticks_per_ms() * millis;
    lapic_write(LAPIC_LVT_TIMER, TIMER_VECTOR | TIMER_MODE_PERIODIC);
    lapic_write(LAPIC_TDCR, 0);
    lapic_write(LAPIC_INIT_COUNT, ticks);
}

fn calibrate_ticks_per_ms() -> u32 {
    lapic_write(LAPIC_TDCR, 0);
    lapic_write(LAPIC_LVT_TIMER, TIMER_VECTOR);
    lapic_write(LAPIC_INIT_COUNT, u32::MAX);

    let mut ctrl: Port<u8> = Port::new(PIT_TIMER_CTRL_PORT);
    unsafe {
        Port::<u8>::new(PIT_COMMAND_PORT).write(0xB4);
        Port::<u8>::new(PIT_CHANNEL2_PORT).write(PIT_LATCH_ONE_MILLISECOND as u8);
        Port::<u8>::new(PIT_CHANNEL2_PORT).write((PIT_LATCH_ONE_MILLISECOND >> 8) as u8);
    }
    loop {
        if unsafe { ctrl.read() } & PIT_OUT2_MASK == 0 {
            break;
        }
    }
    loop {
        if unsafe { ctrl.read() } & PIT_OUT2_MASK != 0 {
            break;
        }
    }
    u32::MAX - lapic_read(LAPIC_CURRENT_COUNT)
}

fn enable_local_apic() {
    unsafe {
        let base = rdmsr(IA32_APIC_BASE_MSR);
        wrmsr(
            IA32_APIC_BASE_MSR,
            (base | APIC_BASE_GLOBAL_ENABLE) & !APIC_BASE_DISABLE_EXEC,
        );
    }
}

fn disable_legacy_pic() {
    unsafe {
        Port::<u8>::new(PIC1_IMR_PORT).write(0xFF);
        Port::<u8>::new(PIC2_IMR_PORT).write(0xFF);
    }
}

fn program_ioapic_redirections() {
    let version = ioapic_read(IO_APIC_VERSION_SELECT);
    let entry_count = ((version >> 16) & 0xFF) + 1;
    for irq in 0..entry_count {
        let entry = ioapic_read_entry(irq) | IO_APIC_RTE_MASKED;
        ioapic_write_entry(irq, entry);
    }
    ioapic_write_entry(
        1,
        0x21 | IO_APIC_RTE_MASKED | IO_APIC_RTE_LEVEL | destination_field(),
    );
}

fn destination_field() -> u64 {
    ((lapic_read(LAPIC_ID) >> 24) as u64) << 56
}

fn update_source_mask(irq: u32, masked: bool) -> Result {
    if irq > LocalApic::MAX_IRQ {
        return Err(InterruptError::InvalidIrq);
    }
    if irq == 0 {
        let mut lvt = lapic_read(LAPIC_LVT_TIMER);
        if masked {
            lvt |= LVT_MASKED;
        } else {
            lvt &= !LVT_MASKED;
        }
        lapic_write(LAPIC_LVT_TIMER, lvt);
        return Ok(());
    }
    let mut entry = ioapic_read_entry(irq);
    if masked {
        entry |= IO_APIC_RTE_MASKED;
    } else {
        entry &= !IO_APIC_RTE_MASKED;
    }
    ioapic_write_entry(irq, entry);
    Ok(())
}

fn lapic_read(register: u64) -> u32 {
    unsafe {
        ((phys_offset() + LOCAL_APIC_MMIO_BASE + register) as *const u32).read_volatile()
    }
}

fn lapic_write(register: u64, value: u32) {
    unsafe {
        ((phys_offset() + LOCAL_APIC_MMIO_BASE + register) as *mut u32).write_volatile(value);
    }
}

fn ioapic_read(select: u32) -> u32 {
    unsafe {
        let base = (phys_offset() + IO_APIC_MMIO_BASE) as *mut u32;
        base.add(IO_APIC_INDEX_PORT).write_volatile(select);
        base.add(IO_APIC_WINDOW_PORT).read_volatile()
    }
}

fn ioapic_write(select: u32, value: u32) {
    unsafe {
        let base = (phys_offset() + IO_APIC_MMIO_BASE) as *mut u32;
        base.add(IO_APIC_INDEX_PORT).write_volatile(select);
        base.add(IO_APIC_WINDOW_PORT).write_volatile(value);
    }
}

fn ioapic_read_entry(irq: u32) -> u64 {
    let low = ioapic_read(IO_APIC_REDIRECTION_BASE + irq * 2) as u64;
    let high = ioapic_read(IO_APIC_REDIRECTION_BASE + irq * 2 + 1) as u64;
    (high << 32) | low
}

fn ioapic_write_entry(irq: u32, entry: u64) {
    ioapic_write(IO_APIC_REDIRECTION_BASE + irq * 2, entry as u32);
    ioapic_write(IO_APIC_REDIRECTION_BASE + irq * 2 + 1, (entry >> 32) as u32);
}

unsafe fn rdmsr(register: u32) -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") register,
        out("eax") low,
        out("edx") high,
        options(nomem, nostack, preserves_flags),
    );
    ((high as u64) << 32) | low as u64
}

unsafe fn wrmsr(register: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") register,
        in("eax") low,
        in("edx") high,
        options(nomem, nostack, preserves_flags),
    );
}
