use alloc::boxed::Box;
use alloc::vec::Vec;
use core::convert::TryInto;
use core::slice::{from_raw_parts, from_raw_parts_mut};
use future::{Future, TimeFuture};
use crate::file_arena::FileHandle;
use kernel::KERNEL;
use messages::HardwareInterrupt;
use system::message::{Exec, Message, MessageType};
use task_arena::TaskHandle;

#[repr(C)]
struct FileReadWriteMessage {
    handle: FileHandle,
    buffer_ptr: usize,
    buffer_len: usize,
}

#[repr(C)]
struct FileCloseMessage {
    handle: FileHandle,
}

#[inline(always)]
pub fn get_system_time() -> u64 {
    unsafe { (*KERNEL).get_system_time() }
}

#[inline(always)]
pub fn exec(entrypoint: usize) {
    unsafe { (*KERNEL).exec(entrypoint) }
}

#[inline(always)]
pub fn wait(future: Box<dyn Future>) {
    unsafe { (*KERNEL).wait(future) }
}

#[inline(always)]
pub fn task_yield() {
    unsafe { (*KERNEL).task_yield() }
}

#[inline(always)]
pub fn preempt() {
    unsafe { (*KERNEL).preempt() }
}

#[inline(always)]
pub fn switch_to_task(task_handle: TaskHandle) -> TaskHandle {
    unsafe { (*KERNEL).switch_to_task(task_handle) }
}

#[inline(always)]
pub fn enqueue_hardware_interrupt(hardware_interrupt: HardwareInterrupt) {
    unsafe { (*KERNEL).enqueue(hardware_interrupt) }
}

#[inline(always)]
pub(crate) fn terminate_current_task() {
    unsafe { (*KERNEL).terminate_current_task() }
}

#[inline(always)]
pub fn syscall(message: &Message) -> usize {
    unsafe { (*KERNEL).syscall(message) }
}

#[inline(always)]
pub fn handle_syscall(message: &Message) -> usize {
    match message.message_type {
        MessageType::FileRead => handle_file_read(message),
        MessageType::FileWrite => handle_file_write(message),
        MessageType::FileOpen => handle_file_open(message),
        MessageType::FileClose => handle_file_close(message),
        MessageType::Exec => handle_exec(message),
    }
}

fn handle_file_open(message: &Message) -> usize {
    let path = core::str::from_utf8(&message.data).unwrap();
    let result = unsafe { (*KERNEL).file_manager.open(path) };
    match result {
        Ok(handle) => ((handle.index as usize) << 8) | (handle.generation as usize),
        Err(_) => usize::MAX,
    }
}

fn handle_file_close(message: &Message) -> usize {
    let msg: &FileCloseMessage = unsafe { core::mem::transmute(message.data.as_ptr()) };
    let result = unsafe { (*KERNEL).file_manager.close(msg.handle) };
    match result {
        Ok(_) => 0,
        Err(_) => usize::MAX,
    }
}

fn handle_file_read(message: &Message) -> usize {
    let msg: &FileReadWriteMessage = unsafe { core::mem::transmute(message.data.as_ptr()) };
    let user_buf = unsafe { from_raw_parts_mut(msg.buffer_ptr as *mut u8, msg.buffer_len) };
    let result = unsafe { (*KERNEL).file_manager.read(msg.handle, user_buf) };
    match result {
        Ok(bytes_read) => bytes_read,
        Err(_) => usize::MAX,
    }
}

fn handle_file_write(message: &Message) -> usize {
    let msg: &FileReadWriteMessage = unsafe { core::mem::transmute(message.data.as_ptr()) };
    let user_buf = unsafe { from_raw_parts(msg.buffer_ptr as *const u8, msg.buffer_len) };
    let result = unsafe { (*KERNEL).file_manager.write(msg.handle, user_buf) };
    match result {
        Ok(bytes_written) => bytes_written,
        Err(_) => usize::MAX,
    }
}

fn handle_exec(message: &Message) -> usize {
    let data = &message.data;
    match Exec::from_u8(data[0]) {
        Exec::Invalid => 0,
        Exec::ThreadSleep => thread_sleep(data),
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
