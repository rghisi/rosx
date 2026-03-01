use core::alloc::{GlobalAlloc, Layout};
use alloc::boxed::Box;
use alloc::string::String;
use crate::future::TimeFuture;
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::default_output::print;
use system::syscall_numbers::SyscallNum;
use system::future::FutureHandle;
use system::ipc::{IpcReplyFuture, IpcServerHandle};
use system::ipc::IpcSendMessage;
use crate::task::{new_elf_task, new_entrypoint_task};

#[cfg(not(test))]
pub fn handle_syscall(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize {
    match SyscallNum::try_from(num) {
        Ok(SyscallNum::Print) => {
            let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(arg1 as *const u8, arg2)) };
            print(format_args!("{}", s));
            0
        }
        Ok(SyscallNum::Sleep) => {
            let future = Box::new(TimeFuture::new(arg1));
            let handle = services().future_registry
                .borrow_mut()
                .register(future)
                .expect("Failed to register sleep future");
            let _ = kernel().wait_future(handle);
            0
        }
        Ok(SyscallNum::Exec) => {
            let entrypoint = arg1;
            match kernel().schedule(new_entrypoint_task(entrypoint)).ok() {
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
            let _ = kernel().wait_future(handle);
            crate::keyboard::pop_key().map_or(0, |c| c as usize)
        }
        Ok(SyscallNum::WaitFuture) => {
            let handle = FutureHandle { index: (arg1 >> 32) as u32, generation: arg1 as u32 };
            let future = kernel().wait_future(handle).unwrap();
            Box::into_raw(Box::new(future)) as usize
        }
        Ok(SyscallNum::IsFutureCompleted) => {
            let handle = FutureHandle { index: (arg1 >> 32) as u32, generation: arg1 as u32 };
            if kernel().is_future_completed(handle) { 1 } else { 0 }
        }
        Ok(SyscallNum::Alloc) => {
            let Ok(layout) = Layout::from_size_align(arg1, arg2) else { return 0 };
            (unsafe { services().memory_manager.alloc(layout) }) as usize
        }
        Ok(SyscallNum::Dealloc) => {
            let Ok(layout) = Layout::from_size_align(arg2, arg3) else { return 0 };
            unsafe { services().memory_manager.dealloc(arg1 as *mut u8, layout) };
            0
        }
        Ok(SyscallNum::TryReadChar) => {
            crate::keyboard::pop_key().map_or(0, |c| c as usize)
        }
        Ok(SyscallNum::LoadElf) => {
            let elf_ptr = arg1;
            let elf_bytes: &[u8] = unsafe { *Box::from_raw(elf_ptr as *mut &[u8]) };
            match kernel().schedule(new_elf_task(elf_bytes)).ok() {
                Some(handle) => ((handle.index as u64) << 32 | (handle.generation as u64)) as usize,
                None => u64::MAX as usize,
            }
        }
        Ok(SyscallNum::IpcFind) => {
            let service: &str = unsafe { *Box::from_raw(arg1 as *mut &str) };
            let result = services().ipc_manager.borrow().find(service);
            Box::into_raw(Box::new(result)) as usize
        }
        Ok(SyscallNum::IpcSend) => {
            let handle_index = arg1 as u8;
            let handle_generation = arg2 as u8;
            let value = arg3 as u32;
            let ipc_server_handle = IpcServerHandle { index: handle_index, generation: handle_generation };
            let message = IpcSendMessage { value };
            let future_handle = services().ipc_manager.borrow_mut().send(ipc_server_handle, message);
            let future = kernel().wait_future(future_handle).unwrap();
            let ipc_reply_future = *future.as_any().downcast_ref::<IpcReplyFuture>().unwrap();
            Box::into_raw(Box::new(ipc_reply_future)) as usize
        }
        Err(_) => 0,
    }
}
