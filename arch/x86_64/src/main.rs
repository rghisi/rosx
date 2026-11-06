#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;
mod apic;
mod cpu;
mod elf_arch;
mod interrupt_controller;
mod interrupts;
mod terminal_fonts;
mod framebuffer;

use crate::cpu::X86_64;
use crate::elf_arch::X86_64ElfArch;
use crate::framebuffer::FramebufferOutput;
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
use x86_generic::debug_console::QemuDebugConsole;
use x86_generic::port::X86PortIo;

static FB_OUTPUT: FramebufferOutput = FramebufferOutput;
pub static QEMU_OUTPUT: QemuDebugConsole = QemuDebugConsole;
static OUTPUTS: &[&dyn kernel::platform::output::KernelOutput] = &[&FB_OUTPUT, &QEMU_OUTPUT];

static MULTIPLEXED_OUTPUT: MultiplexOutput = MultiplexOutput::new(OUTPUTS);

static CPU: X86_64 = X86_64::new();
static ELF_ARCH: X86_64ElfArch = X86_64ElfArch;
static PORT_IO: X86PortIo = X86PortIo;

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
    let ramdisk = ramdisk_slice(boot_info);
    let memory_blocks = build_memory_blocks(boot_info, phys_offset);
    kernel::kernel::bootstrap(&memory_blocks, &MULTIPLEXED_OUTPUT);
    kprintln!("[KERNEL] Initializing");
    let mut kernel = Kernel::new(&KCONFIG);
    kernel.setup();
    kernel::rom::init(ramdisk);
    let rom = kernel::rom::image();
    let _ = kernel.schedule(FunctionTask::new("[K] Kstatsd", kernel::kstatsd::main));
    match kernel.schedule_with_task_handle(new_elf_task("atad", roster_elf(rom, "atad"))) {
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
    let _ = kernel.schedule(new_elf_task("init", roster_elf(rom, "init")));

    kprintln!("[KERNEL] Starting");
    kernel.start();
    panic!("[KERNEL] Crashed spectacularly, should never reached here.");
}

fn ramdisk_slice(boot_info: &BootInfo) -> &'static [u8] {
    let ramdisk_addr = boot_info
        .ramdisk_addr
        .into_option()
        .expect("[KERNEL] no ramdisk: build the boot medium with the ROM image");
    let ramdisk_len = usize::try_from(boot_info.ramdisk_len).expect("[KERNEL] ramdisk too large");
    assert!(ramdisk_len > 0, "[KERNEL] ramdisk is empty");
    unsafe { core::slice::from_raw_parts(ramdisk_addr as *const u8, ramdisk_len) }
}

fn roster_elf<'a>(rom: &'a [u8], name: &str) -> &'a [u8] {
    kernel::rom::file_in(rom, name).unwrap_or_else(|| panic!("[KERNEL] ROM image has no {name}"))
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
