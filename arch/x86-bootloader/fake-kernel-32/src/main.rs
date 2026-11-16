#![no_std]
#![no_main]

use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::DEBUG_EXIT_SUCCESS;
use rosx_boot_common::debug_exit;
use rosx_boot_common::serial::Serial;

#[unsafe(no_mangle)]
extern "C" fn _start(_eax_magic: u32, _ebx_ptr: u32) -> ! {
    let console = Serial::COM1;
    console.init();
    console.write_line("[FK32] alive");
    debug_exit(DEBUG_EXIT_SUCCESS)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let console = Serial::COM1;
    console.init();
    console.write_line("[FK32] panic");
    debug_exit(DEBUG_EXIT_FAILURE)
}
