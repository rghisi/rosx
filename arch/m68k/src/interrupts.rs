use kernel::kernel::{kernel, kernel_is_ready};
use kernel::kernel_cell::KernelCell;

pub static SYSTEM_TIME_MS: KernelCell<u32> = KernelCell::new(0);

/* ── Goldfish RTC (timer) at 0xff006000 ──────────────────────────────────────
 *
 * The RTC has no periodic mode; it must be re-armed after each interrupt.
 * All MMIO registers are 32-bit.
 */

const RTC_BASE: usize = 0xff006000;
const RTC_TIME_LOW: usize = 0x00;
const RTC_TIME_HIGH: usize = 0x04;
const RTC_ALARM_LOW: usize = 0x08;
const RTC_ALARM_HIGH: usize = 0x0c;
const RTC_IRQ_ENABLED: usize = 0x10;
const RTC_CLEAR_INTERRUPT: usize = 0x1c;

/* ── Goldfish PIC at 0xff005000 (PIC #5 — drives CPU IRQ level 6) ───────────
 *
 * The Goldfish RTC #1 is connected to PIC #5 IRQ 0.
 */

const PIC5_BASE: usize = 0xff005000;
const PIC_REG_IRQ_PENDING: usize = 0x04;
const PIC_REG_DISABLE: usize = 0x0c;
const PIC_REG_ENABLE: usize = 0x10;

const RTC_IRQ_MASK: u32 = 1 << 0;

const TICK_RATE_HZ: u64 = 100;
const NS_PER_TICK: u64 = 1_000_000_000 / TICK_RATE_HZ;
const MS_PER_TICK: u32 = (1_000 / TICK_RATE_HZ) as u32;

fn rtc_read(offset: usize) -> u32 {
    let reg = (RTC_BASE + offset) as *const u32;
    // SAFETY: Goldfish RTC MMIO address is fixed in the QEMU virt machine
    unsafe { core::ptr::read_volatile(reg) }
}

fn rtc_write(offset: usize, value: u32) {
    let reg = (RTC_BASE + offset) as *mut u32;
    // SAFETY: Goldfish RTC MMIO address is fixed in the QEMU virt machine
    unsafe { core::ptr::write_volatile(reg, value) }
}

fn pic5_write(offset: usize, value: u32) {
    let reg = (PIC5_BASE + offset) as *mut u32;
    // SAFETY: Goldfish PIC #5 MMIO address is fixed in the QEMU virt machine
    unsafe { core::ptr::write_volatile(reg, value) }
}

fn rtc_current_time_ns() -> u64 {
    let low = rtc_read(RTC_TIME_LOW) as u64;
    let high = rtc_read(RTC_TIME_HIGH) as u64;
    (high << 32) | low
}

fn rtc_set_alarm(time_ns: u64) {
    rtc_write(RTC_ALARM_HIGH, (time_ns >> 32) as u32);
    // Writing ALARM_LOW arms the alarm
    rtc_write(RTC_ALARM_LOW, time_ns as u32);
}

pub fn init() {
    rtc_write(RTC_IRQ_ENABLED, 1);
    let next = rtc_current_time_ns() + NS_PER_TICK;
    rtc_set_alarm(next);
    pic5_write(PIC_REG_ENABLE, RTC_IRQ_MASK);
}

fn rearm_timer() {
    rtc_write(RTC_CLEAR_INTERRUPT, 1);
    pic5_write(PIC_REG_DISABLE, RTC_IRQ_MASK);
    let next = rtc_current_time_ns() + NS_PER_TICK;
    rtc_set_alarm(next);
    pic5_write(PIC_REG_ENABLE, RTC_IRQ_MASK);
}

#[no_mangle]
extern "C" fn timer_interrupt_handler_rs() {
    *SYSTEM_TIME_MS.borrow_mut() += MS_PER_TICK;
    rearm_timer();
    if kernel_is_ready() {
        kernel().preempt();
    }
}

#[no_mangle]
extern "C" fn syscall_handler(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize {
    kernel::syscall::handle_syscall(num, arg1, arg2, arg3)
}
