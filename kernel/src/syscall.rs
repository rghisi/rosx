use crate::future::{Future, TimeFuture};
use crate::kernel::KERNEL;
use crate::messages::HardwareInterrupt;
use crate::task::TaskHandle;
use alloc::boxed::Box;
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

pub fn handle_syscall(num: u64, arg1: u64, arg2: u64, _arg3: u64) -> usize {
    if num == SyscallNum::Print as u64 {
        let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(arg1 as *const u8, arg2 as usize)) };
        print(format_args!("{}", s));
        return 0;
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
