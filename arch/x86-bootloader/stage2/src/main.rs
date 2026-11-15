#![no_std]
#![no_main]

use rosx_boot_common::serial::Serial;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::DEBUG_EXIT_SUCCESS;
use rosx_boot_common::debug_exit;

mod entry;

const WRAP_BASE: usize = 0x0010_0000;
const PROBE_ADDRESS: usize = 0x0000_0500;
const PROBE_PATTERN_LOW: u8 = 0xC0;
const PROBE_PATTERN_HIGH: u8 = 0x5A;

#[unsafe(no_mangle)]
extern "C" fn stage2_main() -> ! {
    let console = Serial::COM1;
    console.init();
    if a20_is_visible() {
        console.write_line("[STAGE2_32] A20=OK");
        debug_exit(DEBUG_EXIT_SUCCESS)
    }
    console.write_line("[ERR] a20");
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn a20_is_visible() -> bool {
    unsafe {
        let low = PROBE_ADDRESS as *mut u8;
        let high = (PROBE_ADDRESS + WRAP_BASE) as *mut u8;
        let saved_low = low.read_volatile();
        let saved_high = high.read_volatile();
        low.write_volatile(PROBE_PATTERN_LOW);
        high.write_volatile(PROBE_PATTERN_HIGH);
        let independent = low.read_volatile() == PROBE_PATTERN_LOW;
        low.write_volatile(saved_low);
        high.write_volatile(saved_high);
        independent
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    let mut console = Serial::COM1;
    console.init();
    let _ = writeln!(console, "[STAGE2] PANIC: {info}");
    debug_exit(DEBUG_EXIT_FAILURE)
}
