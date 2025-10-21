#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

mod vga_buffer;
mod cpu;
mod debug_console;
mod multi_debug;
mod interrupts;

use alloc::boxed::Box;
use kernel::kprint;
use core::arch::asm;
use core::panic::PanicInfo;
use core::ptr::null;
use buddy_system_allocator::LockedHeap;
use lazy_static::lazy_static;
use spin::Mutex;
use kernel::kprintln;
use kernel::cpu::Cpu;
use kernel::debug::init_debug;
use kernel::kernel::Kernel;
use kernel::function_task::FunctionTask;
use kernel::kconfig::KConfig;
use kernel::task_scheduler_round_robin::RoundRobin;
use kernel::task_scheduler::TaskScheduler;
use kernel::task::Task;
use crate::cpu::X86_64;
use crate::debug_console::QEMU_DEBUG;
use crate::multi_debug::MultiDebugOutput;
use crate::vga_buffer::{Color, ColorCode, Writer, VGA_DEBUG};
use bootloader::BootInfo;

extern crate alloc;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<27> = LockedHeap::<27>::new();

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("\n!!! PANIC !!!");
    if let Some(location) = info.location() {
        kprintln!("Panic at {}:{}:{}", location.file(), location.line(), location.column());
    }
    kprintln!("Message: {}", info.message());
    kprintln!("System halted.");
    loop {}
}

lazy_static! {
    pub static ref WRITER: Mutex<Writer> = Mutex::new(Writer::new(ColorCode::new(Color::Green, Color::Black)));
}

static DEBUG_OUTPUTS: &[&dyn kernel::debug::DebugOutput] = &[
    &VGA_DEBUG,
    &QEMU_DEBUG,
];

static MULTI_DEBUG: MultiDebugOutput = MultiDebugOutput::new(DEBUG_OUTPUTS);

static CPU: X86_64 = X86_64{};

static KCONFIG: KConfig = KConfig {
    cpu: &CPU,
    scheduler: get_scheduler,
    idle_task: new_idle_task
};
fn get_scheduler() -> Box<dyn TaskScheduler> {
    Box::new(RoundRobin::new())
}

fn new_idle_task() -> Box<Task> {
    FunctionTask::new("[K] Idle", idle_job)
}

#[unsafe(no_mangle)]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    init_debug(&MULTI_DEBUG);
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

    let mut kernel = Kernel::new(&KCONFIG);

    kernel.setup();
    kernel.schedule(FunctionTask::new("1", dummy_job1));
    kernel.schedule(FunctionTask::new("2", dummy_job2));
    kernel.schedule(FunctionTask::new("3", dummy_job3));
    kernel.schedule(FunctionTask::new("4", dummy_job4));
    kernel.schedule(FunctionTask::new("5", dummy_job5));
    kernel.start();

    println!("[KERNEL] Oops, should never reached here, crashing spectacularly.");

    loop {
    }
}

fn idle_job() {
    println!("Idle Task Start");
    let mut counter = 0;
    loop {
        if counter % 10 == 0 {
            println!("Idling... {}", counter);
        }
        counter += 1;
        unsafe { asm!("hlt"); }
    }
}

fn dummy_job1() {
    let mut counter = 0;
    print!("1");
    for i in 0..10 {
        counter += 1;
        delay(20000500);
        print!("1");
    }
    print!("1");
}

fn dummy_job2() {
    let mut counter = 10;
    print!("2");
    for i in 0..10 {
        counter -= 1;
        delay(20001000);
        print!("2");
    }
    print!("2");
}

fn dummy_job3() {
    let mut counter = 10 * 10;
    print!("3");
    for i in 0..10 {
        counter -= 10;
        delay(20000300);
        print!("3");
    }
    print!("3");
}

fn dummy_job4() {
    let mut counter = 10 * 2;
    print!("4");
    for i in 0..10 {
        counter -= 2;
        delay(20000700);
        print!("4");
    }
    print!("4");
}

fn dummy_job5() {
    let mut counter = 10 * 5;
    print!("5");
    for i in 0..10 {
        counter -= 5;
        delay(20000000);
        print!("5");
    }
    print!("5");
}

fn delay(ticks: u32) {
    for _ in 0..ticks {
        unsafe { asm!("nop"); }
    }
}


