use rosx_boot_common::bios::Regs32;
use rosx_boot_common::memnorm::FIRST_MEGABYTE;
use rosx_boot_common::memnorm::RawEntry;
use rosx_boot_common::memnorm::normalize;
use rosx_boot_common::memnorm::region_sum;
use rosx_boot_common::memnorm::synthesize_e801;
use rosx_boot_common::memnorm::synthesize_e88;
use rosx_boot_common::memnorm::top_of_usable;
use rosx_boot_common::memnorm::usable_bytes;
use rosx_boot_common::serial::Serial;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::debug_exit;
use system::boot_info::MEMORY_MAP_MAX_ENTRIES;
use system::boot_info::MemoryKind;
use system::boot_info::MemoryRegion;

use crate::read_u32;
use crate::thunk::BiosThunk;

const E820_FUNCTION: u32 = 0xE820;
const E801_FUNCTION: u32 = 0xE801;
const SMAP_SIGNATURE: u32 = 0x534D_4150;
const E820_BOUNCE_SELECTOR: u16 = 0x4000;
const E820_BOUNCE_ADDRESS: usize = 0x0004_0000;
const E820_SLOT_BYTES: usize = 24;
const E820_MIN_ENTRY_BYTES: u32 = 20;
const MEMORY_MAP_HOME: usize = 0x0003_0000;

enum E820Outcome {
    Collected(usize),
    Refused,
    TooMany,
}

pub fn publish_memory_map(thunk: &BiosThunk, console: &Serial, mem88_kib: u16) {
    let mut raw = [RawEntry::usable(0, 0); MEMORY_MAP_MAX_ENTRIES];
    let raw_count = match collect_e820(thunk, console, &mut raw) {
        E820Outcome::Collected(count) => count,
        E820Outcome::TooMany => {
            console.write_line("[ERR] memnorm overflow");
            debug_exit(DEBUG_EXIT_FAILURE);
        }
        E820Outcome::Refused => {
            console.write_line("[MEM] e820 refused, trying e801");
            match detected_e801(thunk) {
            Some((entries, count)) => {
                raw[..count].copy_from_slice(&entries[..count]);
                count
            }
            None => {
                console.write_line("[MEM] e801 refused, using e88");
                let (entries, count) = synthesize_e88(mem88_kib);
                raw[..count].copy_from_slice(&entries[..count]);
                count
            }
        }
        },
    };
    let raw_sum = raw[..raw_count].iter().map(|entry| entry.length).sum();
    let mut map = [MemoryRegion {
        base: 0,
        length: 0,
        kind: MemoryKind::Reserved,
    }; MEMORY_MAP_MAX_ENTRIES];
    let count = match normalize(&raw[..raw_count], None, &mut map) {
        Ok(count) => count,
        Err(_) => {
            console.write_line("[ERR] memnorm");
            debug_exit(DEBUG_EXIT_FAILURE);
        }
    };
    write_map_to_home(&map[..count]);
    for region in &map[..count] {
        console.write_str("[MEM] base=");
        console.write_hex(region.base, 16);
        console.write_str(" len=");
        console.write_hex(region.length, 16);
        console.write_str(" kind=");
        console.write_dec(u64::from(region.kind as u32));
        console.write_line("");
    }
    let usable_above_one_megabyte = map[..count]
        .iter()
        .filter(|region| region.kind == MemoryKind::Usable && region.base >= FIRST_MEGABYTE)
        .map(|region| region.length)
        .sum::<u64>();
    let gap = top_of_usable(&map[..count])
        .saturating_sub(FIRST_MEGABYTE)
        .saturating_sub(usable_above_one_megabyte);
    console.write_str("[MEM] entries=");
    console.write_dec(count as u64);
    console.write_str(" raw_sum=");
    console.write_hex(raw_sum, 16);
    console.write_str(" norm_sum=");
    console.write_hex(region_sum(&map[..count]), 16);
    console.write_str(" usable=");
    console.write_hex(usable_bytes(&map[..count]), 16);
    console.write_str(" gap=");
    console.write_hex(gap, 16);
    console.write_line(" OK");
}

fn collect_e820(thunk: &BiosThunk, console: &Serial, raw: &mut [RawEntry]) -> E820Outcome {
    let mut count = 0usize;
    let mut continuation: u16 = 0;
    loop {
        let slot = E820_BOUNCE_ADDRESS + count * E820_SLOT_BYTES;
        let mut regs = Regs32::new(
            E820_FUNCTION,
            u32::from(continuation),
            E820_MIN_ENTRY_BYTES,
            SMAP_SIGNATURE,
        );
        regs.es = E820_BOUNCE_SELECTOR;
        regs.di = (count * E820_SLOT_BYTES) as u16;
        thunk.call32(&mut regs);
        if regs.carry()
            || regs.ecx < E820_MIN_ENTRY_BYTES
            || regs.eax != SMAP_SIGNATURE
        {
            console.write_str("[E820] idx=");
            console.write_dec(u64::from(continuation));
            console.write_str(" cf=");
            console.write_dec(u64::from(regs.carry() as u8));
            console.write_str(" eax=");
            console.write_hex(u64::from(regs.eax), 8);
            console.write_str(" ecx=");
            console.write_hex(u64::from(regs.ecx), 8);
            console.write_str(" ebx=");
            console.write_hex(u64::from(regs.ebx), 8);
            console.write_line("");
            return E820Outcome::Refused;
        }
        raw[count] = RawEntry {
            base: read_u64(slot),
            length: read_u64(slot + 8),
            kind: read_u32(slot + 16),
        };
        continuation = regs.ebx as u16;
        count += 1;
        if continuation == 0 {
            return E820Outcome::Collected(count);
        }
        if count == MEMORY_MAP_MAX_ENTRIES {
            return E820Outcome::TooMany;
        }
    }
}

fn detected_e801(thunk: &BiosThunk) -> Option<([RawEntry; 3], usize)> {
    let mut regs = Regs32::new(E801_FUNCTION, 0, 0, 0);
    thunk.call32(&mut regs);
    if regs.carry() || regs.eax as u16 == E801_FUNCTION as u16 {
        return None;
    }
    Some(synthesize_e801(regs.eax as u16, regs.ecx as u16))
}

fn write_map_to_home(regions: &[MemoryRegion]) {
    for (index, region) in regions.iter().enumerate() {
        let slot = MEMORY_MAP_HOME + index * core::mem::size_of::<MemoryRegion>();
        write_u64(slot, region.base);
        write_u64(slot + 8, region.length);
        write_u32(slot + 16, region.kind as u32);
    }
}

fn read_u64(address: usize) -> u64 {
    u64::from(read_u32(address)) | (u64::from(read_u32(address + 4)) << 32)
}

fn write_u32(address: usize, value: u32) {
    unsafe { (address as *mut u32).write_volatile(value) }
}

fn write_u64(address: usize, value: u64) {
    unsafe { (address as *mut u64).write_volatile(value) }
}
