#![no_main]
#![no_std]

mod vga_buffer;
mod cpu;
mod debug_console;
mod multi_debug;

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
fn panic(_panic: &PanicInfo) -> ! {
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
    unsafe {
        let stack1: [u8; 1024] = [0; 1024];
        let stack2: [u8; 1024] = [0; 1024];
        let sp1 = stack1.as_ptr().addr();
        let sp12 = stack1.as_ptr().offset(stack1.len() as isize - 1).addr();
        let sp2 = stack2.as_ptr().addr();
        let sp22 = stack2.as_ptr().offset(stack2.len() as isize - 1).addr();
        println!("{} {} {} {}", sp1, sp12, sp2, sp22);
        let a = idle_task.stack_pointer();
        println!("{}", a)
    }
    let cpu: &'static dyn Cpu = &CPU;
    let mut kernel = Kernel::new(cpu, idle_task);


    println!("Init");

    kernel.setup();
    kernel.schedule(FunctionTask::new("1", dummy_job1));
    kernel.schedule(FunctionTask::new("2", dummy_job2));
    kernel.start();

    println!("Oops, should never reached here, crashing spectacularly.");

    loop {
    }
}

fn idle_job() {
    println!("Idle Task Start");
    delay(1000000);
    println!("Idle Task Finish");
}

fn dummy_job1() {
    println!("Job 1 Start");
    delay(1000000);
    println!("Job 1 Finish");
}

fn dummy_job2() {
    println!("Job 2 Start");
    delay(1000000);
    println!("Job 2 Finish");
}

fn delay(ticks: u32) {
    for _ in 0..ticks {
        unsafe { asm!("nop"); }
    }
}


