#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;
mod cpu;
mod debug_console;
mod interrupts;
mod vga_buffer;
mod framebuffer;
mod ansi_parser;

use crate::cpu::X86_64;
use crate::debug_console::QemuDebugConsole;
use crate::framebuffer::FramebufferOutput;
use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use bootloader_api::config::Mapping;
use core::panic::PanicInfo;
use kernel::memory::memory_manager::{MemoryBlock, MemoryBlocks};
use kernel::default_output::MultiplexOutput;
use kernel::kconfig::KConfig;
use kernel::kernel::Kernel;
use kernel::scheduler;
use kernel::kprintln;
use kernel::panic::handle_panic;
use kernel::task::{FunctionTask};

static FB_OUTPUT: FramebufferOutput = FramebufferOutput;
pub static QEMU_OUTPUT: QemuDebugConsole = QemuDebugConsole;
static OUTPUTS: &[&dyn kernel::default_output::KernelOutput] = &[&FB_OUTPUT, &QEMU_OUTPUT];

static MULTIPLEXED_OUTPUT: MultiplexOutput = MultiplexOutput::new(OUTPUTS);

static CPU: X86_64 = X86_64::new();

static KCONFIG: KConfig = KConfig {
    cpu: &CPU,
    scheduler_factory: scheduler::mfq_scheduler,
};

const BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

#[cfg_attr(not(test), panic_handler)]
fn panic(info: &PanicInfo) -> ! {
    handle_panic(info);
}

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    if let Some(fb) = boot_info.framebuffer.as_mut() {
        framebuffer::init(fb.buffer_mut().as_mut_ptr() as u64, fb.info());
    }
    let memory_blocks = build_memory_blocks(boot_info);
    kernel::kernel::bootstrap(&memory_blocks, &MULTIPLEXED_OUTPUT);
    kprintln!("[KERNEL] Initializing");
    let mut kernel = Kernel::new(&KCONFIG);
    kernel.setup();
    // kernel.schedule(FunctionTask::new("1", dummy::app::main));
    // kernel.schedule(FunctionTask::new("2", dummy::app::main2));
    // kernel.schedule(FunctionTask::new("3", dummy::app::main3));
    // kernel.schedule(FunctionTask::new("4", dummy::app::main4));
    let _ = kernel.schedule(FunctionTask::new("RandomServer", kernel::ipc::random_gen_server::main));
    let _ = kernel.schedule(FunctionTask::new("Shell", shell::shell::main));
    // kernel.schedule(FunctionTask::new("6", dummy::app::main_with_wait));
    // kernel.schedule(FunctionTask::new("Test Suite", test_suite::app::main));

    kprintln!("[KERNEL] Starting");
    kernel.start();
    panic!("[KERNEL] Crashed spectacularly, should never reached here.");
}

fn build_memory_blocks(boot_info: &BootInfo) -> MemoryBlocks {
    let mut largest_region_size = 0u64;

    for region in boot_info.memory_regions.iter() {
        if let bootloader_api::info::MemoryRegionKind::Usable = region.kind {
            let size = region.end - region.start;

            if size > largest_region_size {
                largest_region_size = size;
            }
        }
    }

    let mut memory_blocks = MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    };

    let physical_memory_offset = boot_info.physical_memory_offset.into_option().unwrap();

    for region in boot_info.memory_regions.iter() {
        if let bootloader_api::info::MemoryRegionKind::Usable = region.kind {
            let size = (region.end - region.start) as usize;
            let start = (region.start + physical_memory_offset) as usize;
            memory_blocks.blocks[memory_blocks.count] = MemoryBlock { start, size };
            memory_blocks.count += 1;
        }
    }

    memory_blocks
}
