use alloc::string::String;
use system::ipc::IpcServerHandle;

pub(crate) struct IpcServerConnection {
    service: String,
}

impl  IpcServerConnection {

    pub(crate) fn new(service: String) -> IpcServerConnection {
        IpcServerConnection {
            service,
        }
    }

   pub(crate) fn receive(&mut self, handle: &mut IpcServerHandle) -> Result<usize, ()> {
       todo!()
   }

   pub(crate) fn reply(&mut self) -> Result<usize, ()> {
       todo!()
   }
}