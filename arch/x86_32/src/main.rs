#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;

mod cpu;
mod elf_arch;
mod interrupts;

pub static CPU: cpu::X86_32 = cpu::X86_32::new();
pub static ELF_ARCH: elf_arch::X86_32ElfArch = elf_arch::X86_32ElfArch;
pub static PORT_IO: x86_generic::port::X86PortIo = x86_generic::port::X86PortIo;

static KCONFIG: kernel::kconfig::KConfig = kernel::kconfig::KConfig {
    cpu: &CPU,
    elf_arch: &ELF_ARCH,
    port_io: &PORT_IO,
    scheduler_factory: kernel::scheduler::mfq_scheduler,
};

use core::panic::PanicInfo;
use kernel::platform::output::MultiplexOutput;
use kernel::memory::{MemoryBlock, MemoryBlocks};
use kernel::panic::handle_panic;

core::arch::global_asm!(include_str!("boot.S"));

static DEBUG_CONSOLE: x86_generic::debug_console::QemuDebugConsole = x86_generic::debug_console::QemuDebugConsole;
static VGA_OUTPUT: x86_generic::vga_buffer::VgaOutput = x86_generic::vga_buffer::VgaOutput;
static OUTPUTS: &[&dyn kernel::platform::output::KernelOutput] = &[&VGA_OUTPUT, &DEBUG_CONSOLE];
static MULTIPLEXED_OUTPUT: MultiplexOutput = MultiplexOutput::new(OUTPUTS);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    handle_panic(info);
}

const MULTIBOOT_MAGIC: u32 = 0x2BADB002;

#[unsafe(no_mangle)]
extern "C" fn kernel_main(multiboot_magic: u32, multiboot_info: u32) -> ! {
    if multiboot_magic != MULTIBOOT_MAGIC {
        panic!("bad Multiboot magic: {:#x}", multiboot_magic);
    }

    x86_generic::vga_buffer::init(0);
    let raw_blocks = unsafe { x86_generic::multiboot::usable_memory(multiboot_info as *const u8) };
    let modules = unsafe { x86_generic::multiboot::boot_modules(multiboot_info as *const u8) };
    let rom_module = modules
        .list()
        .first()
        .expect("[KERNEL] no ROM module: boot with the ROM image attached");
    let rom_image =
        unsafe { core::slice::from_raw_parts(rom_module.start as *const u8, rom_module.size) };
    let mut memory_blocks = trim_to_safe_memory(raw_blocks);
    memory_blocks.carve(rom_module.start, rom_module.start + rom_module.size);
    kernel::kernel::bootstrap(&memory_blocks, &MULTIPLEXED_OUTPUT);
    kernel::kprintln!("[KERNEL] Initializing");

    let mut kernel = kernel::kernel::Kernel::new(&KCONFIG);
    kernel.setup();
    kernel::rom::init(rom_image);
    let rom = kernel::rom::image();
    let _ = kernel.schedule(kernel::task::FunctionTask::new("[K] Kstatsd", kernel::kstatsd::main));
    match kernel.schedule_with_task_handle(kernel::task::new_elf_task("atad", roster_elf(rom, "atad"))) {
        Ok((atad_handle, _)) => {
            if !kernel::platform::portio::grant(atad_handle, 0x1F0, 0x1F7) {
                kernel::kprintln!("[KERNEL] atad primary taskfile grant failed");
            }
            if !kernel::platform::portio::grant(atad_handle, 0x3F6, 0x3F6) {
                kernel::kprintln!("[KERNEL] atad alternate status grant failed");
            }
        }
        Err(()) => kernel::kprintln!("[KERNEL] atad schedule failed"),
    }
    let _ = kernel.schedule(kernel::task::new_elf_task("init", roster_elf(rom, "init")));
    kernel::kprintln!("[KERNEL] Starting");
    kernel.start();
    panic!("[KERNEL] Crashed spectacularly, should never reached here.");
}

fn roster_elf<'a>(rom: &'a [u8], name: &str) -> &'a [u8] {
    kernel::rom::file_in(rom, name).unwrap_or_else(|| panic!("[KERNEL] ROM image has no {name}"))
}

unsafe extern "C" {
    static kernel_end: u8;
}

fn trim_to_safe_memory(raw: x86_generic::multiboot::BootMemory) -> MemoryBlocks {
    let safe_start = core::ptr::addr_of!(kernel_end) as usize;
    let mut out = MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    };
    for region in raw.regions() {
        if out.count == out.blocks.len() {
            break;
        }
        let region_end = region.start + region.size;
        let start = region.start.max(safe_start);
        if start < region_end {
            out.blocks[out.count] = MemoryBlock { start, size: region_end - start };
            out.count += 1;
        }
    }
    out
}

