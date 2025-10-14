#![no_main]
#![no_std]
#![feature(abi_x86_interrupt)]

mod vga_buffer;
mod cpu;
mod debug_console;
mod multi_debug;
mod interrupts;

use kernel::kprint;
use core::arch::asm;
use core::panic::PanicInfo;
use buddy_system_allocator::LockedHeap;
use lazy_static::lazy_static;
use spin::Mutex;
use kernel::kprintln;
use kernel::cpu::Cpu;
use kernel::debug::init_debug;
use kernel::kernel::Kernel;
use kernel::function_task::FunctionTask;
use crate::cpu::X86_64;
use crate::debug_console::QEMU_DEBUG;
use crate::multi_debug::MultiDebugOutput;
use crate::vga_buffer::{Color, ColorCode, Writer, VGA_DEBUG};

extern crate alloc;

static CPU: X86_64 = X86_64{};

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::<32>::new();

const HEAP_SIZE: usize = 81920;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

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


#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe {
        HEAP_ALLOCATOR.lock().init(&raw mut HEAP as usize, HEAP_SIZE);
    }
    init_debug(&MULTI_DEBUG);

    let idle_task = FunctionTask::new("Idle Task", idle_job);
    let cpu: &'static dyn Cpu = &CPU;
    let mut kernel = Kernel::new(cpu, idle_task);


    println!("[KERNEL] Booting");

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


