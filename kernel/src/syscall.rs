use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::Ordering::Relaxed;
use future::{Future, TimeFuture};
use kernel::KERNEL;
use messages::HardwareInterrupt;
use system::message::{Exec, Message, MessageType};
use task_arena::TaskHandle;

#[inline(always)]
pub fn get_system_time() -> u64 {
    unsafe {
        (*KERNEL).get_system_time()
    }
}

#[inline(always)]
pub fn exec(entrypoint: usize) {
    unsafe {
        (*KERNEL).exec(entrypoint);
    }
}

#[inline(always)]
pub fn wait(future: Box<dyn Future>) {
    unsafe {
        (*KERNEL).wait(future);
    }
}

#[inline(always)]
pub fn task_yield() {
    unsafe {
        (*KERNEL).task_yield();
    }
}

#[inline(always)]
pub fn preempt() {
    unsafe {
        (*KERNEL).preempt();
    }
}

#[inline(always)]
pub fn switch_to_task(task_handle: TaskHandle) -> TaskHandle {
    unsafe {
        (*KERNEL).switch_to_task(task_handle)
    }
}

#[inline(always)]
pub fn enqueue_hardware_interrupt(hardware_interrupt: HardwareInterrupt) {
    unsafe {
        (*KERNEL).enqueue(hardware_interrupt);
    }
}

#[inline(always)]
pub(crate) fn terminate_current_task() {
    unsafe {
        (*KERNEL).terminate_current_task();
    }
}

#[inline(always)]
pub fn syscall(message: &Message) -> usize {
    unsafe {
        (*KERNEL).syscall(message)
    }
}

#[inline(always)]
pub fn handle_syscall(message: &Message) -> usize {
    match message.message_type {
        MessageType::FileRead => {1}
        MessageType::FileWrite => {2}
        MessageType::FileOpen => {3}
        MessageType::FileClose => {4}
        MessageType::Exec => {
            handle_exec(message)
        }
    }
}

fn handle_exec(message: &Message) -> usize {
    let data = &message.data;
    match Exec::from_u8(data[0]) {
        Exec::Invalid => { 0 }
        Exec::ThreadSleep => { thread_sleep(data) }
    }
}

fn thread_sleep(data: &Vec<u8>) -> usize {
    let n0 = data[1] as u64;
    let n1 = data[2] as u64;
    let n2 = data[3] as u64;
    let n3 = data[4] as u64;
    let ms = n0 << 24 | n1 << 16 | n2 << 8 | n3;
    let future = Box::new(TimeFuture::new(ms));
    wait(future);
    0
}