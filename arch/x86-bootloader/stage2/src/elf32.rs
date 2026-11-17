extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use fs_core::engine::Engine;
use fs_core::engine::MetadataView;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::debug_exit;
use rosx_boot_common::elf32::ElfError;
use rosx_boot_common::elf32::KernelImage;
use rosx_boot_common::elf32::KernelSegment;
use rosx_boot_common::elf32::plan_kernel_image;
use rosx_boot_common::debugport::DebugConsole;
use system::boot_info::BOOT_INFO_MAGIC;

use crate::fat::BootVolume;
use crate::fat::refuse_engine;

const KERNEL_PATH: &[u8] = b"/KERNEL.ELF";
const HEADER_WINDOW: u64 = 4096;
const PLACEMENT_WINDOW: u64 = 16 * 512;
const HANDOFF_STACK_TOP: usize = 0x0007_F000;
const PAGE_ENABLE_BIT: u32 = 1 << 31;
const INTERRUPT_ENABLE_BIT: u32 = 1 << 9;

pub fn load_kernel(console: &DebugConsole, volume: &mut BootVolume) -> KernelImage {
    let metadata = Engine::lookup(volume, KERNEL_PATH).unwrap_or_else(|error| refuse_engine(console, error));
    let handle = Engine::open(volume, KERNEL_PATH).unwrap_or_else(|error| refuse_engine(console, error));
    let table = read_window(console, volume, handle, 0, metadata.size().min(HEADER_WINDOW) as usize);
    let image = plan_kernel_image(&table).unwrap_or_else(|error| refuse_elf(console, error));
    for segment in &image.segments[..image.segment_count] {
        place_segment(console, volume, handle, metadata.size(), *segment);
    }
    Engine::close(volume, handle).unwrap_or_else(|error| refuse_engine(console, error));
    report(console, &image);
    image
}

fn read_window(
    console: &DebugConsole,
    volume: &mut BootVolume,
    handle: rxfat::FatHandle,
    offset: u64,
    want: usize,
) -> Vec<u8> {
    let mut bytes = vec![0u8; want];
    let mut done = 0usize;
    while done < want {
        let count = Engine::read(volume, handle, offset + done as u64, &mut bytes[done..])
            .unwrap_or_else(|error| refuse_engine(console, error));
        if count == 0 {
            refuse_layout(console, "truncated");
        }
        done += count;
    }
    bytes
}

fn place_segment(
    console: &DebugConsole,
    volume: &mut BootVolume,
    handle: rxfat::FatHandle,
    file_size: u64,
    segment: KernelSegment,
) {
    if segment.file_offset.saturating_add(segment.file_bytes) > file_size {
        refuse_layout(console, "truncated");
    }
    let mut placed = 0u64;
    while placed < segment.file_bytes {
        let span = (segment.file_bytes - placed).min(PLACEMENT_WINDOW) as usize;
        let destination = unsafe {
            core::slice::from_raw_parts_mut((segment.memory_address + placed) as *mut u8, span)
        };
        let count = Engine::read(volume, handle, segment.file_offset + placed, destination)
            .unwrap_or_else(|error| refuse_engine(console, error));
        if count == 0 {
            refuse_layout(console, "short-read");
        }
        placed += count as u64;
    }
    let zero_fill = (segment.memory_bytes - segment.file_bytes) as usize;
    if zero_fill > 0 {
        unsafe {
            core::ptr::write_bytes(
                (segment.memory_address + segment.file_bytes) as *mut u8,
                0,
                zero_fill,
            );
        }
    }
}

pub fn enter(console: &DebugConsole, image: &KernelImage, boot_info: u64) -> ! {
    if read_cr0() & PAGE_ENABLE_BIT != 0 {
        refuse_state(console, "cr0.pg");
    }
    if read_eflags() & INTERRUPT_ENABLE_BIT != 0 {
        refuse_state(console, "eflags.if");
    }
    let entry = image.entry as u32;
    let magic = BOOT_INFO_MAGIC;
    let pointer = boot_info as u32;
    unsafe {
        core::arch::asm!(
            "mov esp, {stack}",
            "mov eax, {return_trap}",
            "push eax",
            "mov eax, {magic}",
            "mov ebx, {boot_info}",
            "jmp {entry}",
            stack = in(reg) (HANDOFF_STACK_TOP + 4) as u32,
            return_trap = sym kernel_returned,
            magic = in(reg) magic,
            boot_info = in(reg) pointer,
            entry = in(reg) entry,
            out("eax") _,
            out("ebx") _,
        )
    }
    rosx_boot_common::halt_forever()
}

fn kernel_returned() -> ! {
    let console = DebugConsole;
    console.write_line("[ERR] kernel returned");
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn read_cr0() -> u32 {
    let value: u32;
    unsafe {
        core::arch::asm!(
            "mov {value}, cr0",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags)
        )
    };
    value
}

fn read_eflags() -> u32 {
    let value: u32;
    unsafe {
        core::arch::asm!("pushfd", "pop {value}", value = out(reg) value, options(preserves_flags))
    };
    value
}

fn refuse_state(console: &DebugConsole, detail: &str) -> ! {
    console.write_str("[ERR] state ");
    console.write_line(detail);
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn report(console: &DebugConsole, image: &KernelImage) {
    console.write_str("[ELF] arch=X86_32 entry=");
    console.write_hex(image.entry, 8);
    console.write_str(" phys=");
    console.write_hex(image.phys_start, 8);
    console.write_str("..");
    console.write_hex(image.phys_end, 8);
    console.write_line("");
}

fn refuse_elf(console: &DebugConsole, error: ElfError) -> ! {
    console.write_str("[ERR] elf ");
    console.write_line(elf_kind(error));
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn elf_kind(error: ElfError) -> &'static str {
    match error {
        ElfError::Class => "class",
        ElfError::Machine => "machine",
        ElfError::Format => "format",
        ElfError::Layout => "layout",
        ElfError::Entry => "entry",
    }
}

fn refuse_layout(console: &DebugConsole, kind: &str) -> ! {
    console.write_str("[ERR] elf ");
    console.write_line(kind);
    debug_exit(DEBUG_EXIT_FAILURE)
}
