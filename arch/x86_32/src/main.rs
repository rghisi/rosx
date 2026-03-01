#![no_main]
#![no_std]

mod debug_console;

use core::panic::PanicInfo;
use kernel::memory::memory_manager::{MemoryBlock, MemoryBlocks};
use kernel::panic::handle_panic;

core::arch::global_asm!(include_str!("boot.S"));

static DEBUG_CONSOLE: debug_console::QemuDebugConsole = debug_console::QemuDebugConsole;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    handle_panic(info);
}

const MULTIBOOT2_MAGIC: u32 = 0x36d76289;

#[unsafe(no_mangle)]
extern "C" fn kernel_main(multiboot_magic: u32, multiboot_info: u32) -> ! {
    if multiboot_magic != MULTIBOOT2_MAGIC {
        panic!("bad Multiboot2 magic: {:#x}", multiboot_magic);
    }

    let memory_blocks = parse_memory_map(multiboot_info as *const u8);
    kernel::kernel::bootstrap(&memory_blocks, &DEBUG_CONSOLE);
    kernel::kprintln!("[x86] Bootstrapped");

    loop {
        unsafe { core::arch::asm!("hlt") };
    }
}

fn parse_memory_map(info_ptr: *const u8) -> MemoryBlocks {
    let mut memory_blocks = MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    };

    // Multiboot2 info layout: [total_size: u32][reserved: u32][tags...]
    let total_size = unsafe { (info_ptr as *const u32).read_unaligned() } as usize;
    let mut offset = 8usize;

    while offset < total_size {
        let tag_ptr = unsafe { info_ptr.add(offset) };
        let tag_type = unsafe { (tag_ptr as *const u32).read_unaligned() };
        let tag_size = unsafe { (tag_ptr as *const u32).add(1).read_unaligned() } as usize;

        if tag_type == 0 {
            break;
        }

        if tag_type == 6 {
            // Memory map tag layout: [type][size][entry_size: u32][entry_version: u32][entries...]
            // Each entry: [base_addr: u64][length: u64][type: u32][reserved: u32]
            let entry_size = unsafe { (tag_ptr as *const u32).add(2).read_unaligned() } as usize;
            let entries_end = unsafe { tag_ptr.add(tag_size) };
            let mut entry_ptr = unsafe { tag_ptr.add(16) };

            while entry_ptr < entries_end && memory_blocks.count < memory_blocks.blocks.len() {
                let base = unsafe { (entry_ptr as *const u64).read_unaligned() };
                let length = unsafe { (entry_ptr as *const u64).add(1).read_unaligned() };
                let mem_type = unsafe { entry_ptr.add(16).cast::<u32>().read_unaligned() };

                if mem_type == 1 && base + length <= usize::MAX as u64 {
                    memory_blocks.blocks[memory_blocks.count] =
                        MemoryBlock { start: base as usize, size: length as usize };
                    memory_blocks.count += 1;
                }

                entry_ptr = unsafe { entry_ptr.add(entry_size) };
            }
        }

        // Tags are padded to 8-byte alignment
        offset += (tag_size + 7) & !7;
    }

    memory_blocks
}
