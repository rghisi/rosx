use fs_core::engine::Engine;
use fs_core::engine::MetadataView;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::debug_exit;
use rosx_boot_common::elf64;
use rosx_boot_common::elf64::KernelImage;
use rosx_boot_common::elf64::KernelSegment;
use rosx_boot_common::gdt;
use rosx_boot_common::memnorm::align_up_page;
use rosx_boot_common::memnorm::place_payload;
use rosx_boot_common::pagetable::ENTRIES_PER_TABLE;
use rosx_boot_common::pagetable::PAGE_TABLE_SET_BYTES;
use rosx_boot_common::pagetable::TABLES_PER_SET;
use rosx_boot_common::pagetable::fill_identity_map;
use rosx_boot_common::debugport::DebugConsole;
use system::boot_info::BOOT_INFO_MAGIC;

use crate::fat::BootVolume;
use crate::fat::refuse_engine;

const EXTENDED_LEAF_FLOOR: u32 = 0x8000_0000;
const EXTENDED_FEATURES_LEAF: u32 = 0x8000_0001;
const LONG_MODE_BIT: u32 = 1 << 29;

const KERNEL_PATH: &[u8] = b"/KERNEL.ELF";
const HEADER_WINDOW: usize = 4096;
const PLACEMENT_WINDOW: usize = 16 * 512;

const PAGE_ENABLE_BIT: u32 = 1 << 31;
const INTERRUPT_ENABLE_BIT: u32 = 1 << 9;

const _: () = assert!(BOOT_INFO_MAGIC == 0x524F_5358);

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u32,
}

static GDT64: [u64; 5] = [
    gdt::NULL,
    gdt::code64(),
    gdt::data32(),
    gdt::user_data(),
    gdt::user_code64(),
];

pub fn ensure_long_mode(console: &DebugConsole) {
    if !cpuid_available() || !long_mode_supported() {
        console.write_line("[ERR] CPU does not support x86_64 Long Mode");
        debug_exit(DEBUG_EXIT_FAILURE);
    }
}

fn long_mode_supported() -> bool {
    cpuid(EXTENDED_LEAF_FLOOR).0 >= EXTENDED_FEATURES_LEAF
        && cpuid(EXTENDED_FEATURES_LEAF).3 & LONG_MODE_BIT != 0
}

fn cpuid_available() -> bool {
    let mut original = 0u32;
    let mut toggled = 0u32;
    unsafe {
        core::arch::asm!(
            "pushfd",
            "pop {original}",
            "mov {toggled}, {original}",
            "xor {toggled}, 0x200000",
            "push {toggled}",
            "popfd",
            "pushfd",
            "pop {toggled}",
            "push {original}",
            "popfd",
            original = inout(reg) original,
            toggled = inout(reg) toggled,
        )
    };
    original != toggled
}

#[inline(never)]
#[unsafe(link_section = ".longmode.text")]
fn cpuid(leaf: u32) -> (u32, u32, u32, u32) {
    let (eax, ebx, ecx, edx): (u32, u32, u32, u32);
    unsafe {
        core::arch::asm!(
            "cpuid",
            inout("eax") leaf => eax,
            lateout("ebx") ebx,
            lateout("ecx") ecx,
            lateout("edx") edx,
        )
    };
    (eax, ebx, ecx, edx)
}

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

pub fn load_kernel(console: &DebugConsole, volume: &mut BootVolume) -> KernelImage {
    let metadata = Engine::lookup(volume, KERNEL_PATH).unwrap_or_else(|error| refuse_engine(console, error));
    let handle = Engine::open(volume, KERNEL_PATH).unwrap_or_else(|error| refuse_engine(console, error));
    let window = usize::try_from(metadata.size()).unwrap_or(HEADER_WINDOW).min(HEADER_WINDOW);
    let table = read_window(console, volume, handle, 0, window);
    let image = elf64::plan_kernel_image(&table).unwrap_or_else(|error| refuse_elf(console, error));
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
        let span = (segment.file_bytes - placed).min(PLACEMENT_WINDOW as u64) as usize;
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

fn report(console: &DebugConsole, image: &KernelImage) {
    console.write_str("[ELF] arch=X86_64 entry=");
    console.write_hex(image.entry, 16);
    console.write_str(" phys=");
    console.write_hex(image.phys_start, 8);
    console.write_str("..");
    console.write_hex(image.phys_end, 8);
    console.write_line("");
}

fn refuse_elf(console: &DebugConsole, error: rosx_boot_common::ElfError) -> ! {
    console.write_str("[ERR] elf ");
    console.write_line(elf_kind(error));
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn elf_kind(error: rosx_boot_common::ElfError) -> &'static str {
    match error {
        rosx_boot_common::ElfError::Class => "class",
        rosx_boot_common::ElfError::Machine => "machine",
        rosx_boot_common::ElfError::Format => "format",
        rosx_boot_common::ElfError::Layout => "layout",
        rosx_boot_common::ElfError::Entry => "entry",
    }
}

fn refuse_layout(console: &DebugConsole, kind: &str) -> ! {
    console.write_str("[ERR] elf ");
    console.write_line(kind);
    debug_exit(DEBUG_EXIT_FAILURE)
}

pub struct PageTables {
    pub root: u64,
    pub end: u64,
}

pub fn build_page_tables(console: &DebugConsole, lower_bound: u64, ceiling: u64) -> PageTables {
    let floor = align_up_page(lower_bound).unwrap_or_else(|| refuse_memory(console));
    let placement = place_payload(floor, PAGE_TABLE_SET_BYTES as u64, ceiling)
        .unwrap_or_else(|| refuse_memory(console));
    let set = unsafe { &mut *(placement.address as *mut [[u64; ENTRIES_PER_TABLE]; TABLES_PER_SET]) };
    fill_identity_map(set, placement.address);
    console.write_str("[PAGETABLE] root=");
    console.write_hex(placement.address, 8);
    console.write_str(" end=");
    console.write_hex(placement.end, 8);
    console.write_line("");
    PageTables {
        root: placement.address,
        end: placement.end,
    }
}

#[inline(never)]
#[unsafe(link_section = ".longmode.text")]
pub fn enter_long_mode(console: &DebugConsole, entry: u64, root: u64, boot_info: u64) -> ! {
    if read_cr0() & PAGE_ENABLE_BIT != 0 {
        refuse_state(console, "cr0.pg");
    }
    if read_eflags() & INTERRUPT_ENABLE_BIT != 0 {
        refuse_state(console, "eflags.if");
    }
    let pointer = GdtPointer {
        limit: (core::mem::size_of_val(&GDT64) - 1) as u16,
        base: &GDT64 as *const u64 as u32,
    };
    unsafe {
        core::arch::asm!(
            "lgdt [{gdt}]",
            "mov cr3, {root}",
            "mov ebp, {entry}",
            "mov edi, {info}",
            "mov edx, cr4",
            "or edx, 0x20",
            "mov cr4, edx",
            "mov ecx, 0xC0000080",
            "rdmsr",
            "or eax, 0x100",
            "wrmsr",
            "mov edx, cr0",
            "or edx, 0x80000000",
            "mov cr0, edx",
            "mov eax, 0x8",
            "lea ecx, 2f",
            "push eax",
            "push ecx",
            "retf",
            ".code64",
            "2:",
            "mov eax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov ss, ax",
            "movabs rsp, 0xFFFF80000007F000",
            "movabs rax, 0xFFFF800000000000",
            "or rbp, rax",
            "or rdi, rax",
            "mov eax, 0x524F5358",
            "jmp rbp",
            gdt = in(reg) &pointer as *const GdtPointer as u32,
            root = in(reg) root as u32,
            entry = in(reg) entry as u32,
            info = in(reg) boot_info as u32,
            options(noreturn)
        )
    }
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
    unsafe { core::arch::asm!("pushfd", "pop {value}", value = out(reg) value, options(preserves_flags)) };
    value
}

fn refuse_state(console: &DebugConsole, detail: &str) -> ! {
    console.write_str("[ERR] state ");
    console.write_line(detail);
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn refuse_memory(console: &DebugConsole) -> ! {
    console.write_line("[ERR] mem");
    debug_exit(DEBUG_EXIT_FAILURE)
}
