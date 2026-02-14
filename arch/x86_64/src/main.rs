#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

extern crate alloc;
mod cpu;
mod debug_console;
mod idle;
mod interrupts;
mod vga_buffer;
mod ansi_parser;

use crate::cpu::X86_64;
use crate::debug_console::QemuDebugConsole;
use crate::idle::idle_task_factory;
use crate::vga_buffer::VgaOutput;
use bootloader::BootInfo;
use core::panic::PanicInfo;
use kernel::allocator::MEMORY_ALLOCATOR;
use system::memory::MemoryRegion;
use kernel::default_output::MultiplexOutput;
use kernel::function_task::FunctionTask;
use kernel::kconfig::KConfig;
use kernel::kernel::Kernel;
use kernel::kprintln;
use kernel::panic::handle_panic;
use usrlib::println;

static VGA_OUTPUT: VgaOutput = VgaOutput;
pub static QEMU_OUTPUT: QemuDebugConsole = QemuDebugConsole;
static OUTPUTS: &[&dyn kernel::default_output::KernelOutput] = &[&VGA_OUTPUT, &QEMU_OUTPUT];

static MULTIPLEXED_OUTPUT: MultiplexOutput = MultiplexOutput::new(OUTPUTS);

static CPU: X86_64 = X86_64::new();

static KCONFIG: KConfig = KConfig {
    cpu: &CPU,
    idle_task_factory,
};

#[cfg_attr(not(test), panic_handler)]
fn panic(info: &PanicInfo) -> ! {
    handle_panic(info);
}

#[unsafe(no_mangle)]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    let (regions, count) = discover_memory_regions(boot_info);
    kernel::kernel::bootstrap(&regions[..count], &MULTIPLEXED_OUTPUT);
    kprintln!("[KERNEL] Initializing - {}", MEMORY_ALLOCATOR.used());
    let mut kernel = Kernel::new(&KCONFIG);
    kernel.setup();
    // kernel.schedule(FunctionTask::new("1", dummy::app::main));
    // kernel.schedule(FunctionTask::new("2", dummy::app::main2));
    // kernel.schedule(FunctionTask::new("3", dummy::app::main3));
    // kernel.schedule(FunctionTask::new("4", dummy::app::main4));
    // kernel.schedule(FunctionTask::new("5", shell::shell::main));
    // kernel.schedule(FunctionTask::new("6", dummy::app::main_with_wait));
    kernel.schedule(FunctionTask::new("Test Suite", test_suite::app::main));
    // kernel.schedule(FunctionTask::new("OOM Test", oom_test));

    static HELLO_ELF: &[u8] = include_bytes!("../../../apps/hello_elf/target/rosx-user/release/hello_elf");
    let elf_task = kernel::elf::load_elf(HELLO_ELF).expect("Failed to load ELF");
    kernel.schedule(elf_task);

    let used = MEMORY_ALLOCATOR.used();
    kprintln!("[KERNEL] Starting - {}", used);
    kernel.start();
    panic!("[KERNEL] Crashed spectacularly, should never reached here.");
}

fn oom_test() {
    kprintln!("[OOM Test] Starting deliberate OOM...");
    let mut v = alloc::vec::Vec::new();
    loop {
        // Allocate 1MB at a time
        v.push(alloc::boxed::Box::new([0u8; 1024 * 1024]));
        kprintln!("[OOM Test] Allocated 1MB, total used: {}", MEMORY_ALLOCATOR.used());
    }
}

const MAX_MEMORY_REGIONS: usize = 32;

fn discover_memory_regions(boot_info: &BootInfo) -> ([MemoryRegion; MAX_MEMORY_REGIONS], usize) {
    let mut regions = [MemoryRegion::new(0, 0); MAX_MEMORY_REGIONS];
    let mut count = 0;
    for region in boot_info.memory_map.iter() {
        let size = region.range.end_addr() - region.range.start_addr();
        if let bootloader::bootinfo::MemoryRegionType::Usable = region.region_type {
            let start = (region.range.start_addr() + boot_info.physical_memory_offset) as usize;
            regions[count] = MemoryRegion::new(start, size as usize);
            count += 1;
        }
    }

    (regions, count)
}


