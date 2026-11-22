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
    port_pool: x86_generic::port::X86_PORT_POOL,
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

#[unsafe(no_mangle)]
extern "C" fn kernel_main(loader_magic: u32, loader_info: u32) -> ! {
    x86_generic::vga_buffer::init(0);
    kernel::platform::output::setup_default_output(&MULTIPLEXED_OUTPUT);
    let memory_blocks = if loader_magic == system::boot_info::BOOT_INFO_MAGIC {
        boot_info_memory(loader_info as usize)
    } else {
        panic!("unknown boot magic: {loader_magic:#010x}: the only supported loader is the RosX stage chain");
    };
    kernel::kernel::bootstrap(&memory_blocks, &MULTIPLEXED_OUTPUT);
    kernel::kprintln!("[KERNEL] Initializing");

    let mut kernel = kernel::kernel::Kernel::new(&KCONFIG);
    kernel.setup();
    static ROM_IMAGE: &[u8] = include_bytes!("../../../target/rosx-i486-user/rom.img");
    kernel::rom::init(ROM_IMAGE);
    let _ = kernel.schedule(kernel::task::FunctionTask::new("[K] Kstatsd", kernel::kstatsd::main), kernel::future::FutureOwner::Kernel);
    static INIT_ELF: &[u8] = include_bytes!("../../../target/rosx-i486-user/release/init");
    let boot_plan = system::boot_plan::BootPlan::new()
        .service("rom:atad", ata_protocol::wire::SERVICE_NAME)
        .service("rom:blockd", block_protocol::manager::wire::SERVICE_NAME)
        .run("rom:attach", &["ATA"])
        .service("rom:vfsd", vfs_protocol::wire::SERVICE_NAME)
        .service("rom:inputd", input_protocol::wire::SERVICE_NAME)
        .service("rom:ps2d", input_protocol::wire::SOURCE_NAME_PS2KEYBOARD)
        .run("rom:mount", &["ata0d0p0", "/"])
        .service("/randomd", "RANDOM")
        .service("/echo", "BULKECHO")
        .service("/pong", "PING")
        .service("/pcid", pci_protocol::wire::SERVICE_NAME)
        .service("/ne2kd", "NE2K")
        .exec("/shell", &[])
        .exec("rom:shell", &[]);
    if let Err(error) = kernel.load_elf_program("init", INIT_ELF, boot_plan.encode(), kernel::future::FutureOwner::Kernel) {
        panic!("init ELF rejected: {error:?}");
    }
    kernel::kprintln!("[KERNEL] Starting");
    kernel.start();
    panic!("[KERNEL] Crashed spectacularly, should never reached here.");
}

unsafe extern "C" {
    static kernel_end: u8;
}

fn boot_info_memory(address: usize) -> MemoryBlocks {
    let safe_start = core::ptr::addr_of!(kernel_end) as usize;
    let mut blocks = empty_blocks();
    match unsafe {
        kernel::platform::bootinfo::usable_blocks(
            address,
            system::boot_info::BootArch::X86_32,
            safe_start,
            &mut blocks,
        )
    } {
        Ok(facts) => {
            kernel::kprintln!(
                "[BOOTINFO] regions={} usable={:#x} blocks={} clipped={} dropped={}",
                facts.regions, facts.usable_bytes, facts.accepted, facts.clipped, facts.dropped
            );
            if facts.dropped > 0 {
                kernel::kprintln!(
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
        blocks: core::array::from_fn(|_| MemoryBlock { start: 0, size: 0 }),
        count: 0,
    }
}

