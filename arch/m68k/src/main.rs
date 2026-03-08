#![no_std]
#![no_main]

extern crate alloc;

pub mod goldfish_tty;
pub mod cpu;
pub mod elf_arch;

use core::panic::PanicInfo;
use kernel::kprintln;
use kernel::panic::handle_panic;
use kernel::memory::{MemoryBlock, MemoryBlocks};
use kernel::kconfig::KConfig;
use kernel::kernel::Kernel;
use kernel::scheduler;

use crate::cpu::M68040;
use crate::goldfish_tty::{GoldfishTty, GOLDFISH_TTY_BASE};
use crate::elf_arch::M68kElfArch;
use kernel::cpu::Cpu;

static CPU: M68040 = M68040::new();
static ELF_ARCH: M68kElfArch = M68kElfArch;
static SERIAL: GoldfishTty = GoldfishTty::new(GOLDFISH_TTY_BASE);

static KCONFIG: KConfig = KConfig {
    cpu: &CPU,
    elf_arch: &ELF_ARCH,
    scheduler_factory: scheduler::fifo_scheduler,
};

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    handle_panic(info);
}

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main() -> ! {
    // 1. Setup memory blocks
    let mut memory_blocks = MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    };

    unsafe extern "C" {
        static kernel_end: u8;
    }

    let safe_start = core::ptr::addr_of!(kernel_end) as usize;
    let ram_size = 128 * 1024 * 1024;

    memory_blocks.blocks[0] = MemoryBlock {
        start: safe_start,
        size: ram_size - safe_start,
    };
    memory_blocks.count = 1;

    // 2. Bootstrap kernel
    kernel::kernel::bootstrap(&memory_blocks, &SERIAL);

    kprintln!("[M68K] Hello, Motorola 68040!");

    // 3. Initialize and start kernel
    let mut kernel = Kernel::new(&KCONFIG);
    kernel.setup();

    kprintln!("[M68K] Starting Kernel...");
    kernel.start();

    loop {
        <M68040 as Cpu>::halt(&CPU);
    }
}
