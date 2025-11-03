#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

mod vga_buffer;
mod cpu;
mod debug_console;
mod multi_debug;
mod interrupts;

use alloc::boxed::Box;
use core::arch::asm;
use core::panic::PanicInfo;
use buddy_system_allocator::LockedHeap;
use lazy_static::lazy_static;
use spin::Mutex;
use kernel::cpu::Cpu;
use kernel::default_output::setup_default_output;
use kernel::kernel::Kernel;
use kernel::function_task::FunctionTask;
use kernel::kconfig::KConfig;
use kernel::task::Task;
use crate::cpu::X86_64;
use crate::debug_console::QEMU_DEBUG;
use crate::multi_debug::MultiDebugOutput;
use crate::vga_buffer::{Color, ColorCode, Writer, VGA_DEBUG};
use bootloader::BootInfo;
use kernel::task_fifo_queue::TaskFifoQueue;
use kernel::task_queue::TaskQueue;
use usrlib::{ println};

extern crate alloc;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<27> = LockedHeap::<27>::new();

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\n!!! PANIC !!!");
    if let Some(location) = info.location() {
        println!("Panic at {}:{}:{}", location.file(), location.line(), location.column());
    }
    println!("Message: {}", info.message());
    println!("System halted.");
    loop {
        unsafe { asm!("hlt"); };
    }
}

lazy_static! {
    pub static ref WRITER: Mutex<Writer> = Mutex::new(Writer::new(ColorCode::new(Color::Green, Color::Black)));
}

static DEBUG_OUTPUTS: &[&dyn kernel::default_output::KernelOutput] = &[
    &VGA_DEBUG,
    &QEMU_DEBUG,
];

static MULTI_DEBUG: MultiDebugOutput = MultiDebugOutput::new(DEBUG_OUTPUTS);

static CPU: X86_64 = X86_64{};

static KCONFIG: KConfig = KConfig {
    cpu: &CPU,
    user_thread_queue: get_user_thread_queue,
    idle_task: new_idle_task
};
fn get_user_thread_queue() -> Box<dyn TaskQueue> {
    Box::new(TaskFifoQueue::new())
}

fn new_idle_task() -> Box<Task> {
    FunctionTask::new("[K] Idle", idle_job)
}

#[unsafe(no_mangle)]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    setup_default_output(&MULTI_DEBUG);
    CPU.setup();

    println!("[KERNEL] Booting");

    println!("[MEMORY] Physical memory offset: 0x{:x}", boot_info.physical_memory_offset);
    println!("[MEMORY] Memory regions:");

    let mut total_usable = 0u64;
    let mut largest_region_size = 0u64;
    let mut largest_region_start = 0u64;

    for region in boot_info.memory_map.iter() {
        println!("  {:?}: 0x{:x} - 0x{:x} ({} KB)",
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

    println!("[MEMORY] Total usable RAM: {} MB", total_usable / (1024 * 1024));
    println!("[MEMORY] Largest region: {} MB at 0x{:x}",
        largest_region_size / (1024 * 1024),
        largest_region_start
    );

    for region in boot_info.memory_map.iter() {
        if let bootloader::bootinfo::MemoryRegionType::Usable = region.region_type {
            let size = (region.range.end_addr() - region.range.start_addr()) as usize;
            let start = (region.range.start_addr() + boot_info.physical_memory_offset) as usize;
            let end = start + size;
            println!("[MEMORY] Allocating region: {}B at 0x{:x}-0x{:x}", size, start, end);
            unsafe {
                HEAP_ALLOCATOR.lock().init(start, size);
            }
        }
    }


    println!("[MEMORY] Heap initialized successfully!");
    println!("[KERNEL] Initializing");
    let mut kernel = Kernel::new(&KCONFIG);
    kernel.setup();
    kernel.schedule(FunctionTask::new("1", dummy::app::main));
    kernel.schedule(FunctionTask::new("2", dummy::app::main2));
    kernel.schedule(FunctionTask::new("3", dummy::app::main3));
    kernel.schedule(FunctionTask::new("4", dummy::app::main4));
    kernel.schedule(FunctionTask::new("5", shell::shell::main));
    println!("[KERNEL] Starting");
    kernel.start();

    println!("[KERNEL] Oops, should never reached here, crashing spectacularly.");

    loop {
    }
}

fn idle_job() {
    println!("Idle Task Start");
    let mut counter = 0;
    loop {
        if counter % 100 == 0 {
            println!("Idling... {}", counter);
        }
        counter += 1;
        unsafe { asm!("hlt"); }
    }
}
