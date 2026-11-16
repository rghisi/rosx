#![no_std]
#![no_main]

use fs_core::device::BlockDevice;
use rosx_boot_common::bios::BOOT_DRIVE_MEMORY;
use rosx_boot_common::bios::Regs16;
use rosx_boot_common::bios::Regs32;
use rosx_boot_common::biosdisk::GEOMETRY_PACKET_ADDRESS;
use rosx_boot_common::biosdisk::HARD_DRIVE_BASE;
use rosx_boot_common::biosdisk::INT13;
use rosx_boot_common::biosdisk::SECTOR_BYTES;
use rosx_boot_common::serial::Serial;
use rosx_boot_common::DEBUG_EXIT_FAILURE;
use rosx_boot_common::DEBUG_EXIT_SUCCESS;
use rosx_boot_common::debug_exit;
use rxpart::mbr::SIGNATURE;

mod blk;
mod entry;
mod heap;
mod memory;
mod thunk;

use blk::BiosBlockDevice;
use thunk::BiosThunk;

const WRAP_BASE: usize = 0x0010_0000;
const PROBE_ADDRESS: usize = 0x0000_0500;
const PROBE_PATTERN_LOW: u8 = 0xC0;
const PROBE_PATTERN_HIGH: u8 = 0x5A;

const MEM88_REPEAT_CALLS: usize = 20;

#[unsafe(no_mangle)]
extern "C" fn stage2_main() -> ! {
    let console = Serial::COM1;
    console.init();
    if !a20_is_visible() {
        console.write_line("[ERR] a20");
        debug_exit(DEBUG_EXIT_FAILURE);
    }
    let Some(thunk) = BiosThunk::install() else {
        console.write_line("[ERR] thunk");
        debug_exit(DEBUG_EXIT_FAILURE);
    };
    let Some(kib) = contiguous_high_memory_kib(&thunk) else {
        console.write_line("[ERR] mem88");
        debug_exit(DEBUG_EXIT_FAILURE);
    };
    console.write_str("[MEM88] ");
    console.write_dec(u64::from(kib));
    console.write_line(" KiB");
    if !thunk.intact() {
        console.write_line("[ERR] trampoline");
        debug_exit(DEBUG_EXIT_FAILURE);
    }
    if !a20_is_visible() {
        console.write_line("[ERR] a20");
        debug_exit(DEBUG_EXIT_FAILURE);
    }
    report_boot_drive_geometry(&console, &thunk);
    memory::publish_memory_map(&thunk, &console, kib as u16);
    if !thunk.intact() {
        console.write_line("[ERR] trampoline");
        debug_exit(DEBUG_EXIT_FAILURE);
    }
    if boot_drive() >= u16::from(HARD_DRIVE_BASE) {
        bring_up_boot_volume(&console, &thunk, boot_drive() as u8);
        if !thunk.intact() {
            console.write_line("[ERR] trampoline");
            debug_exit(DEBUG_EXIT_FAILURE);
        }
    }
    console.write_line("[STAGE2_32] A20=OK THUNK=OK");
    debug_exit(DEBUG_EXIT_SUCCESS)
}

fn bring_up_boot_volume(console: &Serial, thunk: &BiosThunk, drive: u8) {
    let Some(mut device) = BiosBlockDevice::bring_up(thunk, drive) else {
        console.write_line("[ERR] disk");
        debug_exit(DEBUG_EXIT_FAILURE);
    };
    let mut boot_record = [0u8; SECTOR_BYTES];
    if device.read_block(0, &mut boot_record).is_err() {
        console.write_line("[ERR] disk read");
        debug_exit(DEBUG_EXIT_FAILURE);
    }
    let signature = u16::from_be_bytes(boot_record[SECTOR_BYTES - 2..].try_into().unwrap());
    if boot_record[SECTOR_BYTES - 2..] != SIGNATURE {
        console.write_line("[ERR] disk signature");
        debug_exit(DEBUG_EXIT_FAILURE);
    }
    console.write_str("[DISK] media=");
    console.write_str(device.media_name());
    console.write_str(" slice=");
    console.write_dec(device.slice_sectors());
    console.write_str(" boot=");
    console.write_hex(u64::from(signature), 4);
    console.write_line("");
}

fn contiguous_high_memory_kib(thunk: &BiosThunk) -> Option<u32> {
    let mut answer = 0u32;
    for _ in 0..MEM88_REPEAT_CALLS {
        let mut regs = Regs32::new(0x8800, 0, 0, 0);
        thunk.call32(&mut regs);
        if regs.carry() {
            return None;
        }
        answer = u32::from(regs.eax as u16);
    }
    (1024..=0xFFFF).contains(&answer).then_some(answer)
}

fn report_boot_drive_geometry(console: &Serial, thunk: &BiosThunk) {
    let mut regs = Regs16::new(0x0800, 0, 0, boot_drive());
    regs.di = GEOMETRY_PACKET_ADDRESS as u16;
    thunk.call(INT13, &mut regs);
    if regs.carry() {
        console.write_line("[INT13] no geometry");
        return;
    }
    let (cylinders, heads, sectors) = if regs.ax >> 8 >= 0x30 {
        extended_geometry()
    } else {
        classic_geometry(&regs)
    };
    console.write_str("[INT13] ");
    console.write_dec(u64::from(cylinders));
    console.write_str("c ");
    console.write_dec(u64::from(heads));
    console.write_str("h ");
    console.write_dec(u64::from(sectors));
    console.write_line("s");
}

fn extended_geometry() -> (u32, u32, u32) {
    let sectors = read_u16(GEOMETRY_PACKET_ADDRESS + 2);
    let heads = read_u16(GEOMETRY_PACKET_ADDRESS + 4);
    let blocks = read_u32(GEOMETRY_PACKET_ADDRESS + 8);
    let cylinders = match sectors.checked_mul(heads) {
        Some(tracks) if tracks != 0 => blocks / tracks,
        _ => 0,
    };
    (cylinders, heads, sectors)
}

fn classic_geometry(regs: &Regs16) -> (u32, u32, u32) {
    let cylinders = u32::from(regs.cx >> 8 & 0xFF) | (u32::from(regs.cx >> 14 & 3) << 8);
    (cylinders + 1, u32::from(regs.dx >> 8) + 1, u32::from(regs.cx & 0x3F))
}

fn boot_drive() -> u16 {
    u16::from(read_byte(BOOT_DRIVE_MEMORY))
}

fn read_byte(address: usize) -> u8 {
    unsafe { core::ptr::read_volatile(address as *const u8) }
}

fn read_u16(address: usize) -> u32 {
    u32::from(read_byte(address)) | (u32::from(read_byte(address + 1)) << 8)
}

fn read_u32(address: usize) -> u32 {
    read_u16(address) | (read_u16(address + 2) << 16)
}

fn a20_is_visible() -> bool {
    unsafe {
        let low = PROBE_ADDRESS as *mut u8;
        let high = (PROBE_ADDRESS + WRAP_BASE) as *mut u8;
        let saved_low = low.read_volatile();
        let saved_high = high.read_volatile();
        low.write_volatile(PROBE_PATTERN_LOW);
        high.write_volatile(PROBE_PATTERN_HIGH);
        let independent = low.read_volatile() == PROBE_PATTERN_LOW;
        low.write_volatile(saved_low);
        high.write_volatile(saved_high);
        independent
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    let mut console = Serial::COM1;
    console.init();
    let _ = writeln!(console, "[STAGE2] PANIC: {info}");
    debug_exit(DEBUG_EXIT_FAILURE)
}
