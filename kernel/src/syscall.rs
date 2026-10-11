use core::alloc::{GlobalAlloc, Layout};
use alloc::boxed::Box;
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::default_output::print;
use system::syscall_numbers::SyscallNum;
use system::error::into_reg;
use system::future::FutureHandle;
use system::ipc::{IpcConnectionHandle, IpcBindingHandle, Message, IpcMessageFuture};
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
            let millis = arg1 as u64;
            kernel().sleep(millis);
            0
        }
        Ok(SyscallNum::Exec) => {
            let entrypoint = arg1;
            match kernel().schedule(new_entrypoint_task(entrypoint)).ok() {
                Some(handle) => handle.pack(),
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
            crate::keyboard::register_future_handle(handle);
            let _ = kernel().wait_future(handle);
            crate::keyboard::pop_key().map_or(0, |c| c as usize)
        }
        Ok(SyscallNum::WaitFuture) => {
            let handle = FutureHandle::unpack(arg1 as usize);
            let out_ptr = arg2;
            let future = kernel().wait_future(handle).unwrap();
            match future.as_any().downcast_ref::<IpcMessageFuture>() {
                Some(ipc) => match ipc.result() {
                    Ok(message) if out_ptr != 0 => { unsafe { core::ptr::write(out_ptr as *mut Message, message) }; 0 }
                    Ok(_) => 0,
                    Err(e) => e.to_reg(),
                },
                None => 0,
            }
        }
        Ok(SyscallNum::IsFutureCompleted) => {
            let handle = FutureHandle::unpack(arg1 as usize);
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
                Some(handle) => handle.pack(),
                None => u64::MAX as usize,
            }
        }
        Ok(SyscallNum::IpcConnect) => {
            let service: &str = unsafe { *Box::from_raw(arg1 as *mut &str) };
            into_reg(services().ipc_manager.borrow_mut().connect(service, kernel().execution_state.current_task()))
        }
        Ok(SyscallNum::IpcDisconnect) => {
            let connection_handle = IpcConnectionHandle::unpack(arg1);
            let result = services().ipc_manager.borrow_mut().disconnect(connection_handle);
            0usize
        }
        Ok(SyscallNum::IpcBind) => {
            let service: &str = unsafe { *Box::from_raw(arg1 as *mut &str) };
            into_reg(services().ipc_manager.borrow_mut().bind_service(service))
        }
        Ok(SyscallNum::IpcSendMessage) => {
            let envelope = unsafe { &*(arg1 as *const Message) };
            match services().ipc_manager.borrow_mut().send_message(kernel().execution_state.current_task(), envelope) {
                Ok(()) => 0,
                Err(e) => e.to_reg(),
            }
        }
        Ok(SyscallNum::IpcReceiveMessage) => services().ipc_manager.borrow_mut().receive_message_async(IpcConnectionHandle::unpack(arg1)).pack(),
        Ok(SyscallNum::IpcAcceptMessage) => services().ipc_manager.borrow_mut().accept_message_async(IpcBindingHandle::unpack(arg1)).pack(),
        Err(_) => 0,
    }
}
