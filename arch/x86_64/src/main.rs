#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;
mod cpu;
mod debug_console;
mod interrupts;
mod vga_buffer;
mod ansi_parser;

use crate::cpu::X86_64;
use crate::debug_console::QemuDebugConsole;
use crate::vga_buffer::VgaOutput;
use bootloader::BootInfo;
use core::panic::PanicInfo;
use kernel::memory::memory_manager::{MemoryBlock, MemoryBlocks};
use kernel::default_output::MultiplexOutput;
use kernel::kconfig::KConfig;
use kernel::kernel::Kernel;
use kernel::scheduler;
use kernel::kprintln;
use kernel::panic::handle_panic;
use kernel::task::new_elf_task;

static VGA_OUTPUT: VgaOutput = VgaOutput;
pub static QEMU_OUTPUT: QemuDebugConsole = QemuDebugConsole;
static OUTPUTS: &[&dyn kernel::default_output::KernelOutput] = &[&VGA_OUTPUT, &QEMU_OUTPUT];

static MULTIPLEXED_OUTPUT: MultiplexOutput = MultiplexOutput::new(OUTPUTS);

static CPU: X86_64 = X86_64::new();

static KCONFIG: KConfig = KConfig {
    cpu: &CPU,
    scheduler_factory: scheduler::mfq_scheduler,
};

#[cfg_attr(not(test), panic_handler)]
fn panic(info: &PanicInfo) -> ! {
    handle_panic(info);
}

#[unsafe(no_mangle)]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    let memory_blocks = build_memory_blocks(boot_info);
    kernel::kernel::bootstrap(&memory_blocks, &MULTIPLEXED_OUTPUT);
    kprintln!("[KERNEL] Initializing");
    let mut kernel = Kernel::new(&KCONFIG);
    kernel.setup();
    // kernel.schedule(FunctionTask::new("1", dummy::app::main));
    // kernel.schedule(FunctionTask::new("2", dummy::app::main2));
    // kernel.schedule(FunctionTask::new("3", dummy::app::main3));
    // kernel.schedule(FunctionTask::new("4", dummy::app::main4));
    // kernel.schedule(FunctionTask::new("5", shell::shell::main));
    // kernel.schedule(FunctionTask::new("6", dummy::app::main_with_wait));
    // kernel.schedule(FunctionTask::new("Test Suite", test_suite::app::main));

    // static HELLO_ELF: &[u8] = include_bytes!("../../../apps/hello_elf/target/rosx-user/release/hello_elf");
    // let elf_task = kernel::elf::load_elf(HELLO_ELF).expect("Failed to load ELF");
    // kernel.schedule(elf_task);

    static SNAKE_ELF: &[u8] = include_bytes!("../../../apps/snake/target/rosx-user/release/snake");
    kernel.schedule(new_elf_task(SNAKE_ELF));

    kprintln!("[KERNEL] Starting");
    kernel.start();
    panic!("[KERNEL] Crashed spectacularly, should never reached here.");
}

fn build_memory_blocks(boot_info: &BootInfo) -> MemoryBlocks {
    let mut largest_region_size = 0u64;

    for region in boot_info.memory_map.iter() {
        if let bootloader::bootinfo::MemoryRegionType::Usable = region.region_type {
            let size = region.range.end_addr() - region.range.start_addr();

            if size > largest_region_size {
                largest_region_size = size;
            }
        }
    }

    let mut memory_blocks = MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    };

    for region in boot_info.memory_map.iter() {
        if let bootloader::bootinfo::MemoryRegionType::Usable = region.region_type {
            let size = (region.range.end_addr() - region.range.start_addr()) as usize;
            let start = (region.range.start_addr() + boot_info.physical_memory_offset) as usize;
            memory_blocks.blocks[memory_blocks.count] = MemoryBlock { start, size };
            memory_blocks.count += 1;

        }
    }

    memory_blocks
}
