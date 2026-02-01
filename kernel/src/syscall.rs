use crate::future::{Future, TimeFuture};
use crate::kernel::KERNEL;
use crate::messages::HardwareInterrupt;
use crate::task::TaskHandle;
use alloc::boxed::Box;
use alloc::vec::Vec;
use system::message::{Exec, Message, MessageData, MessageType};
use crate::default_output::print;

use system::syscall_numbers::SyscallNum;

#[inline(always)]
pub fn get_system_time() -> u64 {
    unsafe { (*KERNEL).get_system_time() }
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
    unsafe { (*KERNEL).switch_to_task(task_handle) }
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
    unsafe { (*KERNEL).syscall(message) }
}

pub fn handle_syscall(num: u64, arg1: u64, _arg2: u64, _arg3: u64) -> usize {
    if num == SyscallNum::Print as u64 {
        let message = unsafe { &*(arg1 as *const Message) };
        return handle_message(message);
    } else if num == SyscallNum::Sleep as u64 {
        let future = Box::new(TimeFuture::new(arg1));
        wait(future);
    } else if num == SyscallNum::Exec as u64 {
        exec(arg1 as usize);
    } else if num == SyscallNum::Yield as u64 {
        task_yield();
    }
    0
}

pub fn handle_message(message: &Message) -> usize {
    match message.message_type {
        MessageType::FileRead => 1,
        MessageType::FileWrite => 2,
        MessageType::FileOpen => 3,
        MessageType::FileClose => 4,
        MessageType::Exec => handle_exec_message(message),
    }
}

fn handle_exec_message(message: &Message) -> usize {
    let data = &message.data;
    match data {
        MessageData::Vec { vec } => {
            match Exec::from_u8(vec[0]) {
                Exec::Invalid => 0,
                Exec::ThreadSleep => thread_sleep(vec),
                Exec::Print => 0,
            }
        }
        MessageData::FmtArgs { args } => {
            print(*args);
            0
        }
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
