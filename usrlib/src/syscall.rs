use core::fmt;
use system::syscall_numbers::SyscallNum;
use system::future::FutureHandle;
use system::ipc::Message;
use crate::arch;

pub struct Syscall {}

impl Syscall {
    pub fn exec(entrypoint: usize) -> FutureHandle {
        let raw = arch::raw_syscall(SyscallNum::Exec as u64, entrypoint as u64, 0, 0);
        FutureHandle {
            index: (raw >> 32) as u32,
            generation: raw as u32,
        }
    }

    pub fn task_yield() {
        arch::raw_syscall(SyscallNum::Yield as u64, 0, 0, 0);
    }

    pub fn sleep(ms: u64) {
        arch::raw_syscall(SyscallNum::Sleep as u64, ms, 0, 0);
    }

    pub fn wait_future(handle: FutureHandle) {
        let packed = (handle.index as u64) << 32 | (handle.generation as u64);
        arch::raw_syscall(SyscallNum::WaitFuture as u64, packed, 0, 0);
    }

    pub fn is_future_completed(handle: FutureHandle) -> bool {
        let packed = (handle.index as u64) << 32 | (handle.generation as u64);
        let result = arch::raw_syscall(SyscallNum::IsFutureCompleted as u64, packed, 0, 0);
        result != 0
    }

    pub fn print(args: fmt::Arguments) {
        let s = alloc::fmt::format(args);
        arch::raw_syscall(SyscallNum::Print as u64, s.as_ptr() as u64, s.len() as u64, 0);
    }

    pub fn read_char() -> char {
        let c = arch::raw_syscall(SyscallNum::ReadChar as u64, 0, 0, 0);
        core::char::from_u32(c as u32).unwrap_or('\0')
    }

    pub fn alloc(size: usize, align: usize) -> *mut u8 {
        arch::raw_syscall(SyscallNum::Alloc as u64, size as u64, align as u64, 0) as *mut u8
    }

    pub fn dealloc(ptr: *mut u8, size: usize, align: usize) {
        arch::raw_syscall(SyscallNum::Dealloc as u64, ptr as u64, size as u64, align as u64);
    }

    pub fn ipc_endpoint_create(id: u32) {
        arch::raw_syscall(SyscallNum::IpcEndpointCreate as u64, id as u64, 0, 0);
    }

    pub fn ipc_send(endpoint_id: u32, request: Message) -> Message {
        let mut reply = Message::new(0);
        arch::raw_syscall(
            SyscallNum::IpcSend as u64,
            endpoint_id as u64,
            &request as *const Message as u64,
            &mut reply as *mut Message as u64,
        );
        reply
    }

    pub fn ipc_recv(endpoint_id: u32) -> (u64, Message) {
        let mut msg = Message::new(0);
        let mut token: u64 = 0;
        arch::raw_syscall(
            SyscallNum::IpcRecv as u64,
            endpoint_id as u64,
            &mut msg as *mut Message as u64,
            &mut token as *mut u64 as u64,
        );
        (token, msg)
    }

    pub fn random_next_u64() -> u64 {
        let request = Message::new(system::ipc::random::TAG_NEXT);
        let reply = Self::ipc_send(system::ipc::endpoint::RANDOM, request);
        reply.words[0]
    }

    pub fn ipc_reply(token: u64, reply: Message) {
        arch::raw_syscall(
            SyscallNum::IpcReply as u64,
            token,
            &reply as *const Message as u64,
            0,
        );
    }
}

