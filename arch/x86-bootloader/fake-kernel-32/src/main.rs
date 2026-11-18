#![no_std]
#![no_main]

use core::mem::offset_of;
use core::mem::size_of;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::DEBUG_EXIT_SUCCESS;
use rosx_boot_common::crc32::crc32;
use rosx_boot_common::debug_exit;
use rosx_boot_common::debugport::DebugConsole;
use rosx_boot_common::vga;
use system::boot_info::BOOT_INFO_MAGIC;
use system::boot_info::BOOT_INFO_VERSION;
use system::boot_info::BootArch;
use system::boot_info::BootInfo;
use system::boot_info::BootMethod;
use system::boot_info::DisplayKind;
use system::boot_info::MEMORY_MAP_MAX_ENTRIES;
use system::boot_info::MemoryKind;
use system::boot_info::MemoryRegion;

#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn _start() {
    core::arch::naked_asm!(
        /* Entered by jmp: EAX = BOOT_INFO_MAGIC, EBX = &BootInfo (physical),
           ESP 16-byte aligned. Park the registers as kernel_main arguments. */
        "push ebx",
        "push eax",
        "call {kernel_main}",
        /* kernel_main diverges; a return would violate the handoff contract. */
        "call {divergence}",
        kernel_main = sym kernel_main,
        divergence = sym kernel_returned,
    )
}

extern "C" fn kernel_main(eax_magic: u32, ebx_ptr: u32) -> ! {
    let mut console = Echo {
        machine: DebugConsole,
        human: false,
    };
    if eax_magic != BOOT_INFO_MAGIC {
        violation(&console, "magic");
    }
    let info = ebx_ptr as *const u8;
    if read_u32(info, offset_of!(BootInfo, magic)) != BOOT_INFO_MAGIC
        || read_u16(info, offset_of!(BootInfo, version)) != BOOT_INFO_VERSION
        || read_u16(info, offset_of!(BootInfo, arch)) != BootArch::X86_32 as u16
        || read_u16(info, offset_of!(BootInfo, boot_method)) != BootMethod::Bios as u16
    {
        violation(&console, "header");
    }
    console.human =
        read_u32(info, offset_of!(BootInfo, display.kind)) == DisplayKind::VgaText as u32;
    walk_memory_map(&console, info);
    let initrd_address = read_u64(info, offset_of!(BootInfo, initrd_addr));
    let initrd_size = read_u64(info, offset_of!(BootInfo, initrd_size));
    let checksum = verify_initrd(&console, initrd_address, initrd_size);
    console.write_line("kernel loaded");
    console.write_line("arch: x86_32");
    console.write_str("initrd_crc32: ");
    console.write_hex(u64::from(checksum), 8);
    console.write_line("");
    debug_exit(DEBUG_EXIT_SUCCESS)
}

struct Echo {
    machine: DebugConsole,
    human: bool,
}

impl Echo {
    fn write_str(&self, text: &str) {
        self.machine.write_str(text);
        if self.human {
            vga::append(text);
        }
    }

    fn write_line(&self, text: &str) {
        self.write_str(text);
        self.write_str("\r\n");
    }

    fn write_hex(&self, value: u64, nibbles: u32) {
        self.machine.write_hex(value, nibbles);
        if self.human {
            vga::append_hex(value, nibbles);
        }
    }
}

fn walk_memory_map(console: &Echo, info: *const u8) {
    let map_ptr = read_u64(info, offset_of!(BootInfo, memory_map_ptr)) as *const u8;
    let map_len = read_u32(info, offset_of!(BootInfo, memory_map_len));
    let entry_size = read_u32(info, offset_of!(BootInfo, memory_map_entry_size));
    if entry_size as usize != size_of::<MemoryRegion>()
        || map_len as usize > MEMORY_MAP_MAX_ENTRIES
    {
        violation(console, "map-format");
    }
    let mut usable_total = 0u64;
    let mut previous_end: Option<u64> = None;
    for index in 0..map_len {
        let entry = map_ptr.wrapping_add(index as usize * entry_size as usize);
        let base = read_u64(entry, offset_of!(MemoryRegion, base));
        let length = read_u64(entry, offset_of!(MemoryRegion, length));
        let kind = MemoryKind::from_wire(read_u32(entry, offset_of!(MemoryRegion, kind)));
        let Some(kind) = kind else {
            violation(console, "map-kind");
        };
        if length == 0 {
            violation(console, "map-entry");
        }
        if let Some(previous) = previous_end
            && base < previous
        {
            violation(console, "map-order");
        }
        previous_end = Some(base + length);
        if kind == MemoryKind::Usable {
            usable_total += length;
        }
    }
    if usable_total == 0 {
        violation(console, "map-usable");
    }
}

fn verify_initrd(console: &Echo, address: u64, size: u64) -> u32 {
    if size == 0 {
        if address != 0 {
            violation(console, "initrd-address");
        }
        return crc32(&[]);
    }
    if address == 0 {
        violation(console, "initrd-address");
    }
    let length = usize::try_from(size).unwrap_or_else(|_| violation(console, "initrd-size"));
    let payload = unsafe { core::slice::from_raw_parts(address as *const u8, length) };
    crc32(payload)
}

fn read_u16(base: *const u8, offset: usize) -> u16 {
    unsafe { core::ptr::read_unaligned(base.add(offset) as *const u16) }
}

fn read_u32(base: *const u8, offset: usize) -> u32 {
    unsafe { core::ptr::read_unaligned(base.add(offset) as *const u32) }
}

fn read_u64(base: *const u8, offset: usize) -> u64 {
    unsafe { core::ptr::read_unaligned(base.add(offset) as *const u64) }
}

fn violation(console: &Echo, kind: &str) -> ! {
    console.write_str("[FK32] violation ");
    console.write_line(kind);
    debug_exit(DEBUG_EXIT_FAILURE)
}

fn kernel_returned() -> ! {
    debug_exit(DEBUG_EXIT_FAILURE)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let console = DebugConsole;
    console.write_line("[FK32] panic");
    debug_exit(DEBUG_EXIT_FAILURE)
}
