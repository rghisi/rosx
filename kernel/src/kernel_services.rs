use crate::future::FutureRegistry;
use crate::kernel_cell::KernelCell;
use crate::memory::memory_manager::{MEMORY_MANAGER, MemoryManager};
use crate::once::Once;
use crate::task_manager::TaskManager;

pub(crate) struct KernelServices {
    pub(crate) task_manager: KernelCell<TaskManager>,
    pub(crate) future_registry: KernelCell<FutureRegistry>,
    pub(crate) memory_manager: &'static MemoryManager,
}

static KERNEL_SERVICES: Once<KernelServices> = Once::new();

pub(crate) fn init() {
    KERNEL_SERVICES.call_once(|| KernelServices {
        task_manager: KernelCell::new(TaskManager::new()),
        future_registry: KernelCell::new(FutureRegistry::new()),
        memory_manager: &MEMORY_MANAGER,
    });
}

pub(crate) fn services() -> &'static KernelServices {
    KERNEL_SERVICES.get().expect("KernelServices not initialized")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;

    #[test]
    fn init_and_access_services() {
        init();
        let s = services();

        let task = Task::new(0, "test", 0x1000, 0);
        let handle = s.task_manager.borrow_mut().add_task(task).unwrap();
        assert_eq!(s.task_manager.borrow().get_state(handle), crate::task::TaskState::Created);

        let future = alloc::boxed::Box::new(crate::future::TaskCompletionFuture::new(handle));
        let fh = s.future_registry.borrow_mut().register(future);
        assert!(fh.is_some());
    }
}
