#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;
mod apic;
mod cpu;
mod debug_console;
mod elf_arch;
mod interrupt_controller;
mod interrupts;
mod portio;
mod vga_buffer;
mod terminal_fonts;
mod framebuffer;
mod ansi_parser;

use crate::cpu::X86_64;
use crate::debug_console::QemuDebugConsole;
use crate::elf_arch::X86_64ElfArch;
use crate::framebuffer::FramebufferOutput;
use crate::portio::X86_64PortIo;
use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use bootloader_api::config::Mapping;
use core::panic::PanicInfo;
use kernel::memory::{MemoryBlock, MemoryBlocks};
use kernel::platform::output::MultiplexOutput;
use kernel::kconfig::KConfig;
use kernel::kernel::Kernel;
use kernel::scheduler;
use kernel::kprintln;
use kernel::panic::handle_panic;
use kernel::task::FunctionTask;
use kernel::task::new_elf_task;

static FB_OUTPUT: FramebufferOutput = FramebufferOutput;
pub static QEMU_OUTPUT: QemuDebugConsole = QemuDebugConsole;
static OUTPUTS: &[&dyn kernel::platform::output::KernelOutput] = &[&FB_OUTPUT, &QEMU_OUTPUT];

static MULTIPLEXED_OUTPUT: MultiplexOutput = MultiplexOutput::new(OUTPUTS);

static CPU: X86_64 = X86_64::new();
static ELF_ARCH: X86_64ElfArch = X86_64ElfArch;
static PORT_IO: X86_64PortIo = X86_64PortIo;

static KCONFIG: KConfig = KConfig {
    cpu: &CPU,
    elf_arch: &ELF_ARCH,
    port_io: &PORT_IO,
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
    apic::set_phys_offset(phys_offset);
    let memory_blocks = build_memory_blocks(boot_info, phys_offset);
    kernel::kernel::bootstrap(&memory_blocks, &MULTIPLEXED_OUTPUT);
    kprintln!("[KERNEL] Initializing");
    let mut kernel = Kernel::new(&KCONFIG);
    kernel.setup();
    static ROM_IMAGE: &[u8] = include_bytes!("../../../target/rosx-user/rom.img");
    kernel::rom::init(ROM_IMAGE);
    let _ = kernel.schedule(FunctionTask::new("[K] Kstatsd", kernel::kstatsd::main));
    // kernel.schedule(FunctionTask::new("1", dummy::app::main));
    // kernel.schedule(FunctionTask::new("2", dummy::app::main2));
    // kernel.schedule(FunctionTask::new("3", dummy::app::main3));
    // kernel.schedule(FunctionTask::new("4", dummy::app::main4));
    static RANDOMD_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/randomd");
    let _ = kernel.schedule(new_elf_task(RANDOMD_ELF));
    static ECHO_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/echo");
    let _ = kernel.schedule(new_elf_task(ECHO_ELF));
    static BULK_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/bulk");
    let _ = kernel.schedule(new_elf_task(BULK_ELF));
    static PONG_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/pong");
    let _ = kernel.schedule(new_elf_task(PONG_ELF));
    static VFSD_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/vfsd");
    let _ = kernel.schedule(new_elf_task(VFSD_ELF));
    static INPUTD_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/inputd");
    let _ = kernel.schedule(new_elf_task(INPUTD_ELF));
    static PS2D_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/ps2d");
    let _ = kernel.schedule(new_elf_task(PS2D_ELF));
    static ATAD_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/atad");
    match kernel.schedule_with_task_handle(new_elf_task(ATAD_ELF)) {
        Ok((atad_handle, _)) => {
            if !kernel::platform::portio::grant(atad_handle, 0x1F0, 0x1F7) {
                kprintln!("[KERNEL] atad primary taskfile grant failed");
            }
            if !kernel::platform::portio::grant(atad_handle, 0x3F6, 0x3F6) {
                kprintln!("[KERNEL] atad alternate status grant failed");
            }
        }
        Err(()) => kprintln!("[KERNEL] atad schedule failed"),
    }
    static SHELL_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/shell");
    let _ = kernel.schedule(new_elf_task(SHELL_ELF));
    // kernel.schedule(FunctionTask::new("6", dummy::app::main_with_wait));

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
