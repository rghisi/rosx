pub(crate) mod data_manager;
pub(crate) mod errors;
pub(crate) mod ipc_manager;
pub(crate) mod mailbox_manager;
pub(crate) mod message_future;
mod mailbox;

pub fn deliver_to_service(service: &str, message: system::ipc::Message) -> bool {
    crate::wake_probe::mm_enter("deliver_to_service", "kbd");
    let delivered = crate::kernel_services::services()
        .ipc_manager
        .borrow_mut()
        .deliver_to_service(service, message)
        .is_ok();
    crate::wake_probe::mm_exit();
    delivered
}

