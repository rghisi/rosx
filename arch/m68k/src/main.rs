#![no_main]
#![no_std]
#![feature(asm_experimental_arch)]
#![feature(asm_const)]

extern crate alloc;

mod ansi_parser;
mod cpu;
mod elf_arch;
mod interrupts;
mod serial;

pub static CPU: cpu::M68K = cpu::M68K::new();
pub static ELF_ARCH: elf_arch::M68KElfArch = elf_arch::M68KElfArch;

static KCONFIG: kernel::kconfig::KConfig = kernel::kconfig::KConfig {
    cpu: &CPU,
    elf_arch: &ELF_ARCH,
    scheduler_factory: kernel::scheduler::mfq_scheduler,
};

use core::panic::PanicInfo;
use kernel::memory::{MemoryBlock, MemoryBlocks, MAX_MEMORY_BLOCKS};
use kernel::panic::handle_panic;

static SERIAL: serial::GoldfishSerial = serial::GoldfishSerial::new();

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    handle_panic(info);
}

extern "C" {
    fn get_kernel_end_addr() -> usize;
}

const RAM_SIZE: usize = 32 * 1024 * 1024; // 32 MB, matching QEMU -m 32M

#[no_mangle]
extern "C" fn kernel_main() -> ! {
    let safe_start = unsafe { get_kernel_end_addr() };
    let memory_blocks = MemoryBlocks {
        blocks: {
            let mut blocks = [MemoryBlock { start: 0, size: 0 }; MAX_MEMORY_BLOCKS];
            blocks[0] = MemoryBlock { start: safe_start, size: RAM_SIZE - safe_start };
            blocks
        },
        count: 1,
    };

    kernel::kernel::bootstrap(&memory_blocks, &SERIAL);
    kernel::kprintln!("[m68k] Bootstrapped");

    let mut kernel = kernel::kernel::Kernel::new(&KCONFIG);
    kernel.setup();
    let _ = kernel.schedule(kernel::task::FunctionTask::new("Idle", idle_task));
    kernel.start();
    panic!("[m68k] kernel.start() returned");
}

fn idle_task() {
    loop {
        kernel::kprintln!("[m68k] idle");
        for _ in 0..1_000_000_usize {
            core::hint::spin_loop();
        }
    }
}

#[no_mangle]
pub extern "C" fn abort() -> ! {
    loop {
        // STOP #0x2700 — halt with all interrupts masked
        unsafe { core::arch::asm!(".short 0x4E72, 0x2700", options(nomem, nostack)) };
    }
}
