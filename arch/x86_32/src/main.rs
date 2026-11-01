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
    let raw_blocks = parse_memory_map(multiboot_info as *const u8);
    let memory_blocks = trim_to_safe_memory(raw_blocks);
    kernel::kernel::bootstrap(&memory_blocks, &MULTIPLEXED_OUTPUT);
    kernel::kprintln!("[KERNEL] Initializing");

    let mut kernel = kernel::kernel::Kernel::new(&KCONFIG);
    kernel.setup();
    static ROM_IMAGE: &[u8] = include_bytes!("../../../target/rosx-i686-user/rom.img");
    kernel::rom::init(ROM_IMAGE);
    let _ = kernel.schedule(kernel::task::FunctionTask::new("[K] Kstatsd", kernel::kstatsd::main));
    static RANDOMD_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/randomd");
    let _ = kernel.schedule(kernel::task::new_elf_task(RANDOMD_ELF));
    static ECHO_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/echo");
    let _ = kernel.schedule(kernel::task::new_elf_task(ECHO_ELF));
    static BULK_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/bulk");
    let _ = kernel.schedule(kernel::task::new_elf_task(BULK_ELF));
    static PONG_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/pong");
    let _ = kernel.schedule(kernel::task::new_elf_task(PONG_ELF));
    static VFSD_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/vfsd");
    let _ = kernel.schedule(kernel::task::new_elf_task(VFSD_ELF));
    static INPUTD_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/inputd");
    let _ = kernel.schedule(kernel::task::new_elf_task(INPUTD_ELF));
    static PS2D_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/ps2d");
    let _ = kernel.schedule(kernel::task::new_elf_task(PS2D_ELF));
    static ATAD_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/atad");
    match kernel.schedule_with_task_handle(kernel::task::new_elf_task(ATAD_ELF)) {
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
    static SHELL_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/shell");
    let _ = kernel.schedule(kernel::task::new_elf_task(SHELL_ELF));
    kernel::kprintln!("[KERNEL] Starting");
    kernel.start();
    panic!("[KERNEL] Crashed spectacularly, should never reached here.");
}

unsafe extern "C" {
    static kernel_end: u8;
}

fn trim_to_safe_memory(raw: MemoryBlocks) -> MemoryBlocks {
    let safe_start = core::ptr::addr_of!(kernel_end) as usize;
    let mut out = MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    };
    for i in 0..raw.count {
        let b = raw.blocks[i];
        let region_end = b.start + b.size;
        let start = b.start.max(safe_start);
        if start < region_end {
            out.blocks[out.count] = MemoryBlock { start, size: region_end - start };
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
    let mmap_addr   = unsafe { (info_ptr as *const u32).add(12).read_unaligned() } as usize;

    // Each mmap entry: [size: u32][addr: u64][len: u64][type: u32]
    // `size` covers everything after itself; advance by size + 4 per entry.
    let mut offset = 0;
    while offset < mmap_length && memory_blocks.count < memory_blocks.blocks.len() {
        let entry = (mmap_addr + offset) as *const u8;
        let size     = unsafe { (entry as *const u32).read_unaligned() } as usize;
        let addr     = unsafe { (entry.add(4)  as *const u64).read_unaligned() };
        let len      = unsafe { (entry.add(12) as *const u64).read_unaligned() };
        let mem_type = unsafe {  entry.add(20).cast::<u32>().read_unaligned() };

        if mem_type == 1 && addr + len <= usize::MAX as u64 {
            memory_blocks.blocks[memory_blocks.count] =
                MemoryBlock { start: addr as usize, size: len as usize };
            memory_blocks.count += 1;
        }

        offset += size + 4;
    }

    memory_blocks
}
