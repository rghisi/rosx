use core::alloc::{GlobalAlloc, Layout};
use alloc::boxed::Box;
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::default_output::print;
use system::syscall_numbers::SyscallNum;
use collections::generational_arena::HalfSize;
use system::future::FutureHandle;
use system::ipc::{IpcMessage, IpcConnectionHandle, IpcBindingHandle, IpcBufferHandle};
use system::future::FutureResult;
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
            let future = kernel().wait_future(handle).unwrap();
            let result: FutureResult = future.into_result();
            Box::into_raw(Box::new(result)) as usize
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
            let result = services().ipc_manager.borrow_mut().connect(service);
            Box::into_raw(Box::new(result)) as usize
        }
        Ok(SyscallNum::IpcDisconnect) => {
            let connection_handle = IpcConnectionHandle::new(arg1 as HalfSize, arg2 as HalfSize);
            let result = services().ipc_manager.borrow_mut().disconnect(connection_handle);
            0usize
        }
        Ok(SyscallNum::IpcSend) => {
            let buffer_handle = IpcBufferHandle::unpack(arg3);
            let connection_handle = IpcConnectionHandle::new(arg1 as HalfSize, arg2 as HalfSize);
            let message = IpcMessage {
                buffer_handle,
                connection_handle
            };
            let result = services().ipc_manager.borrow_mut().send_to_server(message);
            Box::into_raw(Box::new(result)) as usize
        }
        Ok(SyscallNum::IpcReceive) => {
            let connection_handle = IpcConnectionHandle::new(arg1 as HalfSize, arg2 as HalfSize);
            services().ipc_manager.borrow_mut().receive_from_server_async(connection_handle).pack()
        }
        Ok(SyscallNum::IpcBind) => {
            let service: &str = unsafe { *Box::from_raw(arg1 as *mut &str) };
            let result = services().ipc_manager.borrow_mut().bind_service(service);
            Box::into_raw(Box::new(result)) as usize
        }
        Ok(SyscallNum::IpcReceiveFromClient) => {
            let binding_handle = IpcBindingHandle::new(arg1 as HalfSize, arg2 as HalfSize);
            services().ipc_manager.borrow_mut().receive_from_all_clients_async(binding_handle).pack()
        }
        Ok(SyscallNum::IpcSendToClient) => {
            let connection_handle = IpcConnectionHandle::new(arg1 as HalfSize, arg2 as HalfSize);
            let message = IpcMessage { buffer_handle: IpcBufferHandle::unpack(arg3), connection_handle };
            let result = services().ipc_manager.borrow_mut().send_to_client(message);
            Box::into_raw(Box::new(result)) as usize
        }
        Ok(SyscallNum::IpcBufferAlloc) => {
            let result = services().ipc_manager.borrow_mut().alloc_buffer();
            Box::into_raw(Box::new(result)) as usize
        }
        Ok(SyscallNum::IpcBufferWrite) => {
            let buffer_handle = IpcBufferHandle::unpack(arg1);
            let bytes = unsafe { core::slice::from_raw_parts(arg2 as *const u8, arg3) };
            let result = services().ipc_manager.borrow_mut().write_buffer(buffer_handle, bytes);
            Box::into_raw(Box::new(result)) as usize
        }
        Ok(SyscallNum::IpcBufferRead) => {
            let buffer_handle = IpcBufferHandle::unpack(arg1);
            let dst = unsafe { core::slice::from_raw_parts_mut(arg2 as *mut u8, arg3) };
            let result = services().ipc_manager.borrow_mut().read_buffer(buffer_handle, dst);
            Box::into_raw(Box::new(result)) as usize
        }
        Ok(SyscallNum::IpcBufferDispose) => {
            let buffer_handle = IpcBufferHandle::unpack(arg1);
            let result = services().ipc_manager.borrow_mut().dispose_buffer(buffer_handle);
            Box::into_raw(Box::new(result)) as usize
        }
        Err(_) => 0,
    }
}
