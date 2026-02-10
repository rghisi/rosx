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
use buddy_system_allocator::LockedHeap;
use core::panic::PanicInfo;
use kernel::allocator::MEMORY_ALLOCATOR;
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

static HEAP_ALLOCATOR: LockedHeap<27> = LockedHeap::<27>::new();

#[unsafe(no_mangle)]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    kernel::kernel::bootstrap(&HEAP_ALLOCATOR, &MULTIPLEXED_OUTPUT);
    initialize_heap(boot_info);
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

fn initialize_heap(boot_info: &BootInfo) {
    kprintln!(
        "[MEMORY] Physical memory offset: 0x{:x}",
        boot_info.physical_memory_offset
    );
    kprintln!("[MEMORY] Memory regions:");

    let mut total_usable = 0u64;
    let mut largest_region_size = 0u64;
    let mut largest_region_start = 0u64;

    for region in boot_info.memory_map.iter() {
        kprintln!(
            "  {:?}: 0x{:x} - 0x{:x} ({} KB)",
            region.region_type,
            region.range.start_addr(),
            region.range.end_addr(),
            (region.range.end_addr() - region.range.start_addr()) / 1024
        );

        if let bootloader::bootinfo::MemoryRegionType::Usable = region.region_type {
            let size = region.range.end_addr() - region.range.start_addr();
            total_usable += size;

            if size > largest_region_size {
                largest_region_size = size;
                largest_region_start = region.range.start_addr();
            }
        }
    }

    kprintln!(
        "[MEMORY] Total usable RAM: {} MB",
        total_usable / (1024 * 1024)
    );
    kprintln!(
        "[MEMORY] Largest region: {} MB at 0x{:x}",
        largest_region_size / (1024 * 1024),
        largest_region_start
    );

    for region in boot_info.memory_map.iter() {
        if let bootloader::bootinfo::MemoryRegionType::Usable = region.region_type {
            let size = (region.range.end_addr() - region.range.start_addr()) as usize;
            let start = (region.range.start_addr() + boot_info.physical_memory_offset) as usize;
            let end = start + size;
            kprintln!(
                "[MEMORY] Allocating region: {}B at 0x{:x}-0x{:x}",
                size, start, end
            );
            unsafe {
                HEAP_ALLOCATOR.lock().init(start, size);
            }
        }
    }
    kprintln!("[MEMORY] Heap initialized successfully!");
}
