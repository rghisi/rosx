#![no_std]
#![no_main]

use core::fmt::Write;
use rosx_boot_common::serial::Serial;
use rosx_boot_common::DEBUG_EXIT_SUCCESS;
use rosx_boot_common::debug_exit;

mod entry;

#[unsafe(no_mangle)]
extern "C" fn stage2_main() -> ! {
    let console = Serial::COM1;
    console.init();
    console.write_line("[STAGE2_32] OK");
    debug_exit(DEBUG_EXIT_SUCCESS)
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut console = Serial::COM1;
    console.init();
    let _ = writeln!(console, "[STAGE2] PANIC: {info}");
    debug_exit(rosx_boot_common::DEBUG_EXIT_FAILURE)
}
