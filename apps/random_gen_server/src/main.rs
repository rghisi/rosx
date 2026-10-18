#![no_std]
#![no_main]

extern crate alloc;

use usrlib::{println};
use usrlib::ipc::ipc_send_value;
use usrlib::syscall::Syscall;

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
                if let Some(fh) = Syscall::ipc_accept_message(binding) {
                    if let Ok(received) = Syscall::wait_for_message(fh) {
                        let value = self.next() as usize;
                        if let Some(connection) = received.connection() {
                            let _ = ipc_send_value(connection, value);
                        }
                    }
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
pub extern "C" fn main(_args_ptr: usize, _args_len: usize) -> isize {
    println!("[IPC Server] Random");
    let mut server = RandomGeneratorServer::new(0xFACADA);
    server.run();
    0
}
