use core::alloc::{GlobalAlloc, Layout};
use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::default_output::print;
use system::syscall_numbers::SyscallNum;
use system::error::ErrorCode;
use system::ipc::Message;
use crate::handle::{FutureHandle, IpcConnectionHandle, IpcBindingHandle, IpcDataHandle};
use crate::memory::free_list_allocator::BlockOwner;
use crate::ipc::message_future::IpcMessageFuture;
use crate::task::{new_elf_task, new_elf_task_with_arguments, new_entrypoint_task, validate_arguments_blob};

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
        Ok(SyscallNum::WaitFuture) => {
            let Some(handle) = FutureHandle::from_packed(arg1 as usize) else { return 0 };
            let out_ptr = arg2;
            let future = kernel().wait_future(handle).unwrap();
            match future.as_any().downcast_ref::<IpcMessageFuture>() {
                Some(ipc) => match ipc.result() {
                    Ok(message) if out_ptr != 0 => { unsafe { core::ptr::write(out_ptr as *mut Message, message) }; 0 }
                    Ok(_) => 0,
                    Err(e) => ErrorCode::from(e).to_reg(),
                },
                None => 0,
            }
        }
        Ok(SyscallNum::IsFutureCompleted) => {
            let Some(handle) = FutureHandle::from_packed(arg1 as usize) else { return 0 };
            if kernel().is_future_completed(handle) { 1 } else { 0 }
        }
        Ok(SyscallNum::Alloc) => {
            let Ok(layout) = Layout::from_size_align(arg1, arg2) else { return 0 };
            let owner = BlockOwner::Task(kernel().execution_state.current_task().pack());
            (unsafe { services().memory_manager.alloc_with_owner(layout, owner) }) as usize
        }
        Ok(SyscallNum::Dealloc) => {
            let Ok(layout) = Layout::from_size_align(arg2, arg3) else { return 0 };
            unsafe { services().memory_manager.dealloc(arg1 as *mut u8, layout) };
            0
        }
        Ok(SyscallNum::LoadElf) => {
            if arg1 == 0 { return u64::MAX as usize; }
            let elf_bytes: &[u8] = unsafe { *Box::from_raw(arg1 as *mut &[u8]) };
            if arg2 == 0 && arg3 == 0 {
                return match kernel().schedule(new_elf_task(elf_bytes)).ok() {
                    Some(handle) => handle.pack(),
                    None => u64::MAX as usize,
                };
            }
            if arg3 > system::args::MAX_ARGUMENT_BYTES { return ErrorCode::ElfArgumentLimitExceeded.to_reg(); }
            if arg2 == 0 || arg3 == 0 { return ErrorCode::ElfArgumentBlobMalformed.to_reg(); }
            let blob = unsafe { core::slice::from_raw_parts(arg2 as *const u8, arg3) };
            if let Err(code) = validate_arguments_blob(blob) { return code.to_reg(); }
            match kernel().schedule(new_elf_task_with_arguments(elf_bytes, Vec::from(blob))).ok() {
                Some(handle) => handle.pack(),
                None => u64::MAX as usize,
            }
        }
        Ok(SyscallNum::IpcConnect) => {
            let service: &str = unsafe { *Box::from_raw(arg1 as *mut &str) };
            match services().ipc_manager.borrow_mut().connect(service, kernel().execution_state.current_task()) {
                Ok(handle) => handle.pack(),
                Err(e) => ErrorCode::from(e).to_reg(),
            }
        }
        Ok(SyscallNum::IpcDisconnect) => {
            if let Some(connection_handle) = IpcConnectionHandle::from_packed(arg1) {
                services().ipc_manager.borrow_mut().disconnect(connection_handle);
            }
            0usize
        }
        Ok(SyscallNum::IpcBind) => {
            let service: &str = unsafe { *Box::from_raw(arg1 as *mut &str) };
            match services().ipc_manager.borrow_mut().bind_service(service) {
                Ok(handle) => handle.pack(),
                Err(e) => ErrorCode::from(e).to_reg(),
            }
        }
        Ok(SyscallNum::IpcSendMessage) => {
            let envelope = unsafe { &*(arg1 as *const Message) };
            match services().ipc_manager.borrow_mut().send_message(kernel().execution_state.current_task(), envelope) {
                Ok(()) => 0,
                Err(e) => ErrorCode::from(e).to_reg(),
            }
        }
        Ok(SyscallNum::IpcReceiveMessage) => match IpcConnectionHandle::from_packed(arg1) {
            Some(connection_handle) => services().ipc_manager.borrow_mut().receive_message_async(connection_handle).pack(),
            None => ErrorCode::Unknown.to_reg(),
        },
        Ok(SyscallNum::IpcAcceptMessage) => match IpcBindingHandle::from_packed(arg1) {
            Some(binding_handle) => services().ipc_manager.borrow_mut().accept_message_async(binding_handle).pack(),
            None => ErrorCode::Unknown.to_reg(),
        },
        Ok(SyscallNum::IpcDataWrite) => {
            let Some(connection) = IpcConnectionHandle::from_packed(arg1) else {
                return ErrorCode::IpcSendConnectionNotFound.to_reg();
            };
            let payload = unsafe { core::slice::from_raw_parts(arg2 as *const u8, arg3) };
            match services().ipc_manager.borrow_mut().write_data(kernel().execution_state.current_task(), connection, payload) {
                Ok(handle) => handle.pack(),
                Err(e) => ErrorCode::from(e).to_reg(),
            }
        }
        Ok(SyscallNum::IpcDataRead) => {
            let Some(token) = IpcDataHandle::from_packed(arg1) else {
                return ErrorCode::IpcDataNotFound.to_reg();
            };
            let dst = unsafe { core::slice::from_raw_parts_mut(arg2 as *mut u8, arg3) };
            match services().ipc_manager.borrow_mut().read_data(kernel().execution_state.current_task(), token, dst) {
                Ok(bytes_copied) => bytes_copied,
                Err(e) => ErrorCode::from(e).to_reg(),
            }
        }
        Ok(SyscallNum::IpcDataRelease) => {
            let Some(token) = IpcDataHandle::from_packed(arg1) else {
                return ErrorCode::IpcDataNotFound.to_reg();
            };
            match services().ipc_manager.borrow_mut().release_data(kernel().execution_state.current_task(), token) {
                Ok(()) => 0,
                Err(e) => ErrorCode::from(e).to_reg(),
            }
        }
        Ok(SyscallNum::RomRead) => {
            let dst = unsafe { core::slice::from_raw_parts_mut(arg2 as *mut u8, arg3) };
            crate::rom::read(arg1 as u64, dst)
        }
        Ok(SyscallNum::PortIn) => {
            let task = kernel().execution_state.current_task();
            match crate::portio::authorize_single(task, arg1, arg2) {
                Err(code) => code.to_reg(),
                Ok(_) if arg3 == 0 => ErrorCode::PortRequestInvalid.to_reg(),
                Ok((port, width)) => {
                    let value = kernel().port_io.read(port, width);
                    unsafe { core::ptr::write(arg3 as *mut u32, value) };
                    0
                }
            }
        }
        Ok(SyscallNum::PortOut) => {
            let task = kernel().execution_state.current_task();
            match crate::portio::authorize_single(task, arg1, arg3) {
                Err(code) => code.to_reg(),
                Ok((port, width)) => {
                    kernel().port_io.write(port, arg2 as u32, width);
                    0
                }
            }
        }
        Ok(SyscallNum::PortInBlock) => {
            let task = kernel().execution_state.current_task();
            match crate::portio::authorize_block(task, arg1, arg3) {
                Err(code) => code.to_reg(),
                Ok((port, count)) => {
                    let destination = unsafe { core::slice::from_raw_parts_mut(arg2 as *mut u16, count) };
                    kernel().port_io.read_words(port, destination);
                    0
                }
            }
        }
        Ok(SyscallNum::PortOutBlock) => {
            let task = kernel().execution_state.current_task();
            match crate::portio::authorize_block(task, arg1, arg3) {
                Err(code) => code.to_reg(),
                Ok((port, count)) => {
                    let source = unsafe { core::slice::from_raw_parts(arg2 as *const u16, count) };
                    kernel().port_io.write_words(port, source);
                    0
                }
            }
        }
        Err(_) => 0,
    }
}
