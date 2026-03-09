#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;
mod cpu;
mod debug_console;
mod elf_arch;
mod interrupts;
mod pci;
mod rtl8139;
mod vga_buffer;
mod terminal_fonts;
mod framebuffer;
mod ansi_parser;

use crate::cpu::X86_64;
use crate::debug_console::QemuDebugConsole;
use crate::elf_arch::X86_64ElfArch;
use crate::framebuffer::FramebufferOutput;
use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use bootloader_api::config::Mapping;
use core::panic::PanicInfo;
use kernel::memory::{MemoryBlock, MemoryBlocks};
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
static ELF_ARCH: X86_64ElfArch = X86_64ElfArch;

static KCONFIG: KConfig = KConfig {
    cpu: &CPU,
    elf_arch: &ELF_ARCH,
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
    let phys_offset = boot_info.physical_memory_offset.into_option().unwrap();
    unsafe { cpu::clear_nx_bits(phys_offset); }
    let memory_blocks = build_memory_blocks(boot_info, phys_offset);
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

    match pci::find_device(0x10EC, 0x8139) {
        Some(dev) => kprintln!("[RTL8139] Found at IRQ {}", dev.irq_line),
        None => kprintln!("[RTL8139] Not found"),
    }

    kprintln!("[KERNEL] Starting");
    kernel.start();
    panic!("[KERNEL] Crashed spectacularly, should never reached here.");
}

fn build_memory_blocks(boot_info: &BootInfo, phys_offset: u64) -> MemoryBlocks {
    let mut memory_blocks = MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    };

    for region in boot_info.memory_regions.iter() {
        if let bootloader_api::info::MemoryRegionKind::Usable = region.kind {
            let size = (region.end - region.start) as usize;
            let start = (region.start + phys_offset) as usize;
            memory_blocks.blocks[memory_blocks.count] = MemoryBlock { start, size };
            memory_blocks.count += 1;
        }
    }

    memory_blocks
}
