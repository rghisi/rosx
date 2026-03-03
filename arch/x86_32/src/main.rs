#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;

mod ansi_parser;
mod cpu;
mod debug_console;
mod elf_arch;
mod interrupts;
mod vga_buffer;

pub static CPU: cpu::X86_32 = cpu::X86_32::new();
pub static ELF_ARCH: elf_arch::X86_32ElfArch = elf_arch::X86_32ElfArch;

static KCONFIG: kernel::kconfig::KConfig = kernel::kconfig::KConfig {
    cpu: &CPU,
    elf_arch: &ELF_ARCH,
    scheduler_factory: kernel::scheduler::mfq_scheduler,
};

use core::panic::PanicInfo;
use kernel::default_output::MultiplexOutput;
use kernel::memory::{MemoryBlock, MemoryBlocks};
use kernel::panic::handle_panic;

core::arch::global_asm!(include_str!("boot.S"));

static DEBUG_CONSOLE: debug_console::QemuDebugConsole = debug_console::QemuDebugConsole;
static VGA_OUTPUT: vga_buffer::VgaOutput = vga_buffer::VgaOutput;
static OUTPUTS: &[&dyn kernel::default_output::KernelOutput] = &[&VGA_OUTPUT, &DEBUG_CONSOLE];
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

    let raw_blocks = parse_memory_map(multiboot_info as *const u8);
    let memory_blocks = trim_to_safe_memory(raw_blocks);
    kernel::kernel::bootstrap(&memory_blocks, &MULTIPLEXED_OUTPUT);
    kernel::kprintln!("[x86] Bootstrapped");

    let mut kernel = kernel::kernel::Kernel::new(&KCONFIG);
    kernel.setup();
    //let _ = kernel.schedule(kernel::task::FunctionTask::new("RandomServer", kernel::ipc::random_gen_server::main));
    let _ = kernel.schedule(kernel::task::FunctionTask::new("Shell", shell::shell::main));
    kernel.start();
    panic!("[x86] kernel.start() returned");
}

unsafe extern "C" {
    static kernel_end: u8;
}

fn trim_to_safe_memory(raw: MemoryBlocks) -> MemoryBlocks {
    let safe_start = unsafe { core::ptr::addr_of!(kernel_end) as usize };
    let mut out = MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    };
    for i in 0..raw.count {
        let b = raw.blocks[i];
        let region_end = b.start + b.size;
        let start = b.start.max(safe_start);
        if start < region_end {
            out.blocks[out.count] = MemoryBlock {
                start,
                size: region_end - start,
            };
            out.count += 1;
        }
    }
    out
}

fn parse_memory_map(info_ptr: *const u8) -> MemoryBlocks {
    let mut memory_blocks = MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    };

    // Multiboot v1 info layout (flat struct):
    //   offset  0: flags (u32)        — bit 6 = mmap fields valid
    //   offset 44: mmap_length (u32)  — byte length of the mmap buffer
    //   offset 48: mmap_addr (u32)    — physical address of the mmap buffer
    let flags = unsafe { (info_ptr as *const u32).read_unaligned() };
    if flags & (1 << 6) == 0 {
        return memory_blocks;
    }

    let mmap_length = unsafe { (info_ptr as *const u32).add(11).read_unaligned() } as usize;
    let mmap_addr = unsafe { (info_ptr as *const u32).add(12).read_unaligned() } as usize;

    // Each mmap entry: [size: u32][addr: u64][len: u64][type: u32]
    // `size` covers everything after itself; advance by size + 4 per entry.
    let mut offset = 0;
    while offset < mmap_length && memory_blocks.count < memory_blocks.blocks.len() {
        let entry = (mmap_addr + offset) as *const u8;
        let size = unsafe { (entry as *const u32).read_unaligned() } as usize;
        let addr = unsafe { (entry.add(4) as *const u64).read_unaligned() };
        let len = unsafe { (entry.add(12) as *const u64).read_unaligned() };
        let mem_type = unsafe { entry.add(20).cast::<u32>().read_unaligned() };

        if mem_type == 1 && addr + len <= usize::MAX as u64 {
            memory_blocks.blocks[memory_blocks.count] = MemoryBlock {
                start: addr as usize,
                size: len as usize,
            };
            memory_blocks.count += 1;
        }

        offset += size + 4;
    }

    memory_blocks
}
