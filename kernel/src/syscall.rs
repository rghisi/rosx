use crate::future::TimeFuture;
use crate::kernel::KERNEL;
use crate::messages::HardwareInterrupt;
use crate::task::TaskHandle;
use alloc::boxed::Box;
use crate::default_output::print;

use system::syscall_numbers::SyscallNum;

use system::future::FutureHandle;

#[inline(always)]
pub fn get_system_time() -> u64 {
    unsafe { (*KERNEL).get_system_time() }
}

#[inline(always)]
pub fn exec(entrypoint: usize) -> Option<FutureHandle> {
    unsafe {
        (*KERNEL).exec(entrypoint).ok()
    }
}

#[inline(always)]
pub fn wait_future(handle: FutureHandle) {
    unsafe {
        (*KERNEL).wait_future(handle);
    }
}

#[inline(always)]
pub fn is_future_completed(handle: FutureHandle) -> bool {
    unsafe {
        (*KERNEL).is_future_completed(handle)
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
        let handle = crate::kernel::FUTURE_REGISTRY
            .register(future)
            .expect("Failed to register sleep future");
        wait_future(handle);
    } else if num == SyscallNum::Exec as u64 {
        if let Some(handle) = exec(arg1 as usize) {
            let packed = (handle.index as u64) << 32 | (handle.generation as u64);
            return packed as usize;
        } else {
            return u64::MAX as usize; // Error code?
        }
    } else if num == SyscallNum::Yield as u64 {
        task_yield();
    } else if num == SyscallNum::ReadChar as u64 {
        if let Some(c) = crate::keyboard::pop_key() {
            return c as usize;
        }

        let future = Box::new(crate::keyboard::KeyboardFuture::new());
        let handle = crate::kernel::FUTURE_REGISTRY
            .register(future)
            .expect("Failed to register keyboard future");
        wait_future(handle);

        if let Some(c) = crate::keyboard::pop_key() {
            return c as usize;
        }
    } else if num == SyscallNum::WaitFuture as u64 {
        let handle = FutureHandle {
            index: (arg1 >> 32) as u32,
            generation: arg1 as u32,
        };
        wait_future(handle);
    } else if num == SyscallNum::IsFutureCompleted as u64 {
        let handle = FutureHandle {
            index: (arg1 >> 32) as u32,
            generation: arg1 as u32,
        };
        return if is_future_completed(handle) { 1 } else { 0 };
    }
    0
}
