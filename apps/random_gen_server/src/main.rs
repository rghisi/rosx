#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};
use core::panic::PanicInfo;
use usrlib::{println};
use usrlib::syscall::Syscall;
use system::future::FutureResult;

struct SyscallAllocator;

unsafe impl GlobalAlloc for SyscallAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Syscall::alloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        Syscall::dealloc(ptr, layout.size(), layout.align());
    }
}

#[global_allocator]
static ALLOCATOR: SyscallAllocator = SyscallAllocator;

struct RandomGeneratorServer {
    state: u32,
}

impl RandomGeneratorServer {
    fn new(seed: u32) -> Self {
        RandomGeneratorServer { state: seed }
    }

    fn run(&mut self) {
        if let Ok(binding) = Syscall::ipc_bind("RANDOM") {
            loop {
                let fh = Syscall::ipc_receive_from_client(binding);
                if let FutureResult::IpcMessage(Ok(msg)) = Syscall::wait_future(fh) {
                    let value = self.next() as usize;
                    let _ = Syscall::ipc_send_to_client(msg.connection_handle, value);
                }
            }
        }
    }

    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        self.state
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    println!("[IPC Server] Random");
    let mut server = RandomGeneratorServer::new(0xFACADA);
    server.run();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    println!("[IPC Server] Random - Panic!");
    loop {

    }
}
