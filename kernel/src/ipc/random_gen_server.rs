use crate::future::TimeFuture;
use crate::ipc::ipc_manager::IpcReplyMessage;
use crate::kernel::kernel;
use crate::kernel_services::services;
use alloc::boxed::Box;
use crate::kprintln;

struct RandomGeneratorServer {
    state: u32,
}

impl RandomGeneratorServer {
    pub fn new(seed: u32) -> Self {
        RandomGeneratorServer { state: seed }
    }

    pub fn run(&mut self) {
        let biding = services()
            .ipc_manager
            .borrow_mut()
            .register("RANDOM")
            .unwrap();
        loop {
            if let Some(message) = services().ipc_manager.borrow_mut().receive(biding) {
                let value = self.next();
                let reply = IpcReplyMessage {
                    value,
                    destination: message.sender,
                    future: message.future,
                };
                services().ipc_manager.borrow_mut().reply(reply);
            } else {
                Self::sleep();
            }
        }
    }

    fn sleep() {
        let future = Box::new(TimeFuture::new(20));
        let handle = services()
            .future_registry
            .borrow_mut()
            .register(future)
            .expect("Failed to register sleep future");
        let _ = kernel().wait_future(handle);
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
