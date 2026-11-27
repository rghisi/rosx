#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;
mod apic;
mod cpu;
mod elf_arch;
#[expect(dead_code, reason = "kept detached for the VESA framebuffer successor plan (plans/x86_64-carrier-swap.md D1)")]
mod framebuffer;
mod interrupt_controller;
mod interrupts;
#[expect(dead_code, reason = "glyph source for the detached framebuffer")]
mod terminal_fonts;

use crate::cpu::X86_64;
use crate::elf_arch::X86_64ElfArch;
use core::panic::PanicInfo;
use kernel::memory::{MemoryBlock, MemoryBlocks};
use kernel::platform::output::MultiplexOutput;
use kernel::kconfig::KConfig;
use kernel::future::FutureOwner;
use kernel::kernel::Kernel;
use kernel::scheduler;
use kernel::kprintln;
use kernel::panic::handle_panic;
use system::boot_plan::BootPlan;
use kernel::task::FunctionTask;
use x86_generic::debug_console::QemuDebugConsole;
use x86_generic::port::X86PortIo;

core::arch::global_asm!(include_str!("boot.S"));

static VGA_OUTPUT: x86_generic::vga_buffer::VgaOutput = x86_generic::vga_buffer::VgaOutput;
pub static QEMU_OUTPUT: QemuDebugConsole = QemuDebugConsole;
static OUTPUTS: &[&dyn kernel::platform::output::KernelOutput] = &[&VGA_OUTPUT, &QEMU_OUTPUT];

static MULTIPLEXED_OUTPUT: MultiplexOutput = MultiplexOutput::new(OUTPUTS);

static CPU: X86_64 = X86_64::new();
static ELF_ARCH: X86_64ElfArch = X86_64ElfArch;
static PORT_IO: X86PortIo = X86PortIo;

static KCONFIG: KConfig = KConfig {
    cpu: &CPU,
    elf_arch: &ELF_ARCH,
    port_io: &PORT_IO,
    port_pool: x86_generic::port::X86_PORT_POOL,
    scheduler_factory: scheduler::mfq_scheduler,
};

#[cfg_attr(not(test), panic_handler)]
fn panic(info: &PanicInfo) -> ! {
    handle_panic(info);
}

#[unsafe(no_mangle)]
extern "C" fn kernel_main(loader_magic: u64, loader_info: u64) -> ! {
    x86_generic::vga_buffer::init(0);
    kernel::platform::output::setup_default_output(&MULTIPLEXED_OUTPUT);
    let memory_blocks = if loader_magic == u64::from(system::boot_info::BOOT_INFO_MAGIC) {
        boot_info_memory(loader_info as usize)
    } else {
        panic!("unknown boot magic: {loader_magic:#x}: the only supported loader is the RosX stage chain");
    };
    kernel::kernel::bootstrap(&memory_blocks, &MULTIPLEXED_OUTPUT);
    kprintln!("[KERNEL] Initializing");
    let mut kernel = Kernel::new(&KCONFIG);
    kernel.setup();
    let initrd = boot_info_initrd(loader_info as usize);
    let files = kernel::rom::validate(initrd)
        .unwrap_or_else(|error| panic!("initrd is not a ROM image: {error:?}"));
    kprintln!(
        "[ROM] addr={:#x} size={} files={}",
        initrd.as_ptr() as usize,
        initrd.len(),
        files
    );
    kernel::rom::init(initrd);
    let _ = kernel.schedule(FunctionTask::new("[K] Kstatsd", kernel::kstatsd::main), FutureOwner::Kernel);
    let init_elf = kernel::rom::program("init").unwrap_or_else(|| panic!("initrd carries no init"));
    let boot_plan = BootPlan::new()
        .service("rom:atad", ata_protocol::wire::SERVICE_NAME)
        .service("rom:blockd", block_protocol::manager::wire::SERVICE_NAME)
        .run("rom:attach", &["ATA"])
        .service("rom:vfsd", vfs_protocol::wire::SERVICE_NAME)
        .service("rom:inputd", input_protocol::wire::SERVICE_NAME)
        .service("rom:ps2d", input_protocol::wire::SOURCE_NAME_PS2KEYBOARD)
        .run("rom:mount", &["ata0d0p0", "/"])
        .service("/srv/pcid", pci_protocol::wire::SERVICE_NAME)
        .service("/srv/ne2kd", "NE2K")
        .exec("/bin/shell", &[])
        .exec("rom:shell", &[]);
    if let Err(error) = kernel.load_elf_program("init", init_elf, boot_plan.encode(), FutureOwner::Kernel) {
        panic!("init ELF rejected: {error:?}");
    }

    kprintln!("[KERNEL] Starting");
    kernel.start();
    panic!("[KERNEL] Crashed spectacularly, should never reached here.");
}

unsafe extern "C" {
    static kernel_phys_end: u8;
}

fn boot_info_initrd(address: usize) -> &'static [u8] {
    match unsafe {
        kernel::platform::bootinfo::initrd(address, system::boot_info::BootArch::X86_64)
    } {
        Ok(Some(slice)) => slice,
        Ok(None) => panic!("no initrd (INITRD.BIN missing on the boot volume)"),
        Err(error) => panic!("initrd refused: {error:?}"),
    }
}

fn boot_info_memory(address: usize) -> MemoryBlocks {
    let safe_start = core::ptr::addr_of!(kernel_phys_end) as usize;
    let mut blocks = empty_blocks();
    match unsafe {
        kernel::platform::bootinfo::usable_blocks(
            address,
            system::boot_info::BootArch::X86_64,
            safe_start,
            &mut blocks,
        )
    } {
        Ok(facts) => {
            kprintln!(
                "[BOOTINFO] regions={} usable={:#x} blocks={} held={} clipped={} dropped={}",
                facts.regions, facts.usable_bytes, facts.accepted, facts.held, facts.clipped, facts.dropped
            );
            if facts.dropped > 0 {
                kprintln!(
                    "[BOOTINFO] warning: {} usable regions dropped for addressability or the {}-block cap",
                    facts.dropped, kernel::memory::MAX_MEMORY_BLOCKS
                );
            }
        }
        Err(error) => panic!("boot info at {address:#x} refused: {error:?}"),
    }
    blocks
}

fn empty_blocks() -> MemoryBlocks {
    MemoryBlocks {
        blocks: core::array::from_fn(|_| MemoryBlock::free(0, 0)),
        count: 0,
    }
}
