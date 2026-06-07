use crate::kernel::kernel;
use crate::kernel_services::services;
use system::ipc::IpcMessage;
use crate::kprintln;

struct RandomGeneratorServer {
    state: u32,
}

impl RandomGeneratorServer {
    pub fn new(seed: u32) -> Self {
        RandomGeneratorServer { state: seed }
    }

    pub fn run(&mut self) {
        if let Ok(binding) = services()
            .ipc_manager
            .borrow_mut()
            .bind_service("RANDOM") {
            loop {
                if let Ok(received_message) = services().ipc_manager.borrow_mut().receive_from_binding(binding) {
                    let value = self.next() as usize;
                    let reply_message = IpcMessage {
                        data: value,
                        connection_handle: received_message.connection_handle,
                    };
                    let _ = services().ipc_manager.borrow_mut().reply(reply_message);
                } else {
                    Self::sleep();
                }
            }
        }
    }

    fn sleep() {
        kernel().sleep(20);
    }

    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        self.state
    }

    fn next_u64(&mut self) -> u64 {
        let hi = self.next() as u64;
        let lo = self.next() as u64;
        (hi << 32) | lo
    }

    fn next_range(&mut self, min: u32, max: u32) -> u32 {
        let range = max - min + 1;
        min + (self.next() % range)
    }
}

pub fn main() {
    kprintln!("[IPC] Starting Random Generation Server");
    let mut server = RandomGeneratorServer::new(0xFACADA);
    server.run();
}
