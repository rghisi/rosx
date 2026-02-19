use core::alloc::{GlobalAlloc, Layout};
use alloc::boxed::Box;
use crate::future::TimeFuture;
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::default_output::print;
use system::syscall_numbers::SyscallNum;
use system::future::FutureHandle;

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
            kernel().wait_future(handle);
            0
        }
        Ok(SyscallNum::Exec) => {
            let entrypoint = arg1 as usize;
            match kernel().exec(entrypoint).ok() {
                Some(handle) => ((handle.index as u64) << 32 | (handle.generation as u64)) as usize,
                None => u64::MAX as usize,
            }
        }
        Ok(SyscallNum::Yield) => {
            kernel().task_yield();
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
            kernel().wait_future(handle);
            crate::keyboard::pop_key().map_or(0, |c| c as usize)
        }
        Ok(SyscallNum::WaitFuture) => {
            let handle = FutureHandle { index: (arg1 >> 32) as u32, generation: arg1 as u32 };
            kernel().wait_future(handle);
            0
        }
        Ok(SyscallNum::IsFutureCompleted) => {
            let handle = FutureHandle { index: (arg1 >> 32) as u32, generation: arg1 as u32 };
            if kernel().is_future_completed(handle) { 1 } else { 0 }
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
