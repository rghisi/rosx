use core::alloc::{GlobalAlloc, Layout};

use crate::future::TimeFuture;
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::messages::HardwareInterrupt;
use crate::task::TaskHandle;
use alloc::boxed::Box;
use crate::default_output::print;

use system::syscall_numbers::SyscallNum;

use system::future::FutureHandle;

#[inline(always)]
pub fn get_system_time() -> u64 {
    kernel().get_system_time()
}

#[inline(always)]
pub fn exec(entrypoint: usize) -> Option<FutureHandle> {
    kernel().exec(entrypoint).ok()
}

#[inline(always)]
pub fn wait_future(handle: FutureHandle) {
    kernel().wait_future(handle);
}

#[inline(always)]
pub fn is_future_completed(handle: FutureHandle) -> bool {
    kernel().is_future_completed(handle)
}

#[inline(always)]
pub fn task_yield() {
    kernel().task_yield();
}

#[inline(always)]
pub fn preempt() {
    kernel().preempt();
}

#[inline(always)]
pub fn switch_to_task(task_handle: TaskHandle) -> TaskHandle {
    kernel().switch_to_task(task_handle)
}

#[inline(always)]
pub fn enqueue_hardware_interrupt(hardware_interrupt: HardwareInterrupt) {
    kernel().enqueue(hardware_interrupt);
}

#[inline(always)]
pub(crate) fn terminate_current_task() {
    kernel().terminate_current_task();
}

#[cfg(not(test))]
pub fn handle_syscall(num: u64, arg1: u64, arg2: u64, _arg3: u64) -> usize {
    match SyscallNum::try_from(num) {
        Ok(SyscallNum::Print) => {
            let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(arg1 as *const u8, arg2 as usize)) };
            print(format_args!("{}", s));
            0
        }
        Ok(SyscallNum::Sleep) => {
            let future = Box::new(TimeFuture::new(arg1));
            let handle = services().future_registry
                .borrow_mut()
                .register(future)
                .expect("Failed to register sleep future");
            wait_future(handle);
            0
        }
        Ok(SyscallNum::Exec) => {
            match exec(arg1 as usize) {
                Some(handle) => ((handle.index as u64) << 32 | (handle.generation as u64)) as usize,
                None => u64::MAX as usize,
            }
        }
        Ok(SyscallNum::Yield) => {
            task_yield();
            0
        }
        Ok(SyscallNum::ReadChar) => {
            if let Some(c) = crate::keyboard::pop_key() {
                return c as usize;
            }
            let future = Box::new(crate::keyboard::KeyboardFuture::new());
            let handle = services().future_registry
                .borrow_mut()
                .register(future)
                .expect("Failed to register keyboard future");
            wait_future(handle);
            crate::keyboard::pop_key().map_or(0, |c| c as usize)
        }
        Ok(SyscallNum::WaitFuture) => {
            let handle = FutureHandle { index: (arg1 >> 32) as u32, generation: arg1 as u32 };
            wait_future(handle);
            0
        }
        Ok(SyscallNum::IsFutureCompleted) => {
            let handle = FutureHandle { index: (arg1 >> 32) as u32, generation: arg1 as u32 };
            if is_future_completed(handle) { 1 } else { 0 }
        }
        Ok(SyscallNum::Alloc) => {
            let Ok(layout) = Layout::from_size_align(arg1 as usize, arg2 as usize) else { return 0 };
            (unsafe { services().memory_manager.alloc(layout) }) as usize
        }
        Ok(SyscallNum::Dealloc) => {
            let Ok(layout) = Layout::from_size_align(arg2 as usize, _arg3 as usize) else { return 0 };
            unsafe { services().memory_manager.dealloc(arg1 as *mut u8, layout) };
            0
        }
        Err(_) => 0,
    }
}
