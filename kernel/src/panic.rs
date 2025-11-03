use core::panic::PanicInfo;
use kprintln;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("\n!!! PANIC !!!");
    if let Some(location) = info.location() {
        kprintln!("Panic at {}:{}:{}", location.file(), location.line(), location.column());
    }
    kprintln!("Message: {}", info.message());
    kprintln!("System halted.");
    loop {

    }
}