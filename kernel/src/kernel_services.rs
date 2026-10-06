use crate::future::FutureRegistry;
use crate::ipc::ipc_manager::IpcManager;
use crate::ipc::mailbox_manager::MailboxManager;
use crate::kernel_cell::KernelCell;
use crate::memory::memory_manager::{MEMORY_MANAGER, MemoryManager};
use crate::once::Once;
use crate::scheduler::fifo_strategy::FifoStrategy;
use crate::scheduler::Scheduler;
use crate::scheduler::SchedulerPorts;
use crate::scheduler::TimerManager;
use crate::task_manager::TaskManager;
use crate::{ForNotifyingFutures, ForWakingTasks, ForCompletingExpiredTimers};
use system::future::{self, Future, FutureHandle};
use system::ipc::{IpcMessage, IpcMessageFuture, IpcReceiveError};
use alloc::boxed::Box;
use alloc::vec::Vec;

// === Port implementations (UseCases) ===

struct SchedulerWakerUseCase {
    scheduler: &'static KernelCell<Scheduler>,
}

impl ForWakingTasks for SchedulerWakerUseCase {
    fn wake_tasks(&self, handles: Vec<crate::task::TaskHandle>) {
        self.scheduler.borrow_mut().wake_tasks(handles);
    }
}

pub(crate) struct FutureRegistryNotifierUseCase {
    pub(crate) future_registry: &'static KernelCell<FutureRegistry>,
}

impl ForNotifyingFutures for FutureRegistryNotifierUseCase {
    fn register(&self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle> {
        self.future_registry.borrow_mut().register(future)
    }

    fn notify(&self, handle: FutureHandle) {
        self.future_registry.borrow_mut().notify(handle);
    }

    fn complete_ipc_message(&self, handle: FutureHandle, message: IpcMessage) {
        if let Ok(future_box) = self.future_registry.borrow_mut().borrow_mut(handle) {
            if let Some(ipc_future) = future_box.as_any_mut().downcast_mut::<IpcMessageFuture>() {
                ipc_future.complete(message);
            }
        }
    }
}

struct FutureRegistryTimerUseCase {
    future_registry: &'static KernelCell<FutureRegistry>,
}

impl ForCompletingExpiredTimers for FutureRegistryTimerUseCase {
    fn complete_timer_future(&self, handle: FutureHandle) {
        if let Ok(future) = self.future_registry.borrow_mut().borrow_mut(handle) {
            future.complete();
        }
        self.future_registry.borrow_mut().notify(handle);
    }
}

// === KernelServices ===

pub(crate) struct KernelServices {
    pub(crate) task_manager: &'static KernelCell<TaskManager>,
    pub(crate) future_registry: &'static KernelCell<FutureRegistry>,
    pub(crate) ipc_manager: &'static KernelCell<IpcManager>,
    pub(crate) timer_manager: &'static KernelCell<TimerManager>,
    pub(crate) scheduler: &'static KernelCell<Scheduler>,
    pub(crate) memory_manager: &'static MemoryManager,
    pub(crate) timer_handler: &'static dyn ForCompletingExpiredTimers,
}

static KERNEL_SERVICES: Once<KernelServices> = Once::new();

pub(crate) fn init() {
    let create_services = || {
        // Leak all KernelCells so the use cases can hold safe &'static
        // references to them. KernelCell<T> contains an UnsafeCell<T> with
        // no Drop, so leaking the wrapper is safe — the T lives for the
        // duration of the process (KERNEL_SERVICES is a static).
        let task_manager = Box::leak(Box::new(KernelCell::new(TaskManager::new())));
        let scheduler_cell = Box::leak(Box::new(KernelCell::new(Scheduler::new(FifoStrategy::new(), SchedulerPorts::noop()))));
        let wake_controller = Box::leak(Box::new(SchedulerWakerUseCase {
            scheduler: scheduler_cell,
        })) as &'static dyn ForWakingTasks;
        let future_registry = Box::leak(Box::new(KernelCell::new(
            FutureRegistry::new_with_wake_controller(wake_controller),
        )));
        let notifier = Box::leak(Box::new(FutureRegistryNotifierUseCase {
            future_registry: future_registry,
        })) as &'static dyn ForNotifyingFutures;
        let timer_handler = Box::leak(Box::new(FutureRegistryTimerUseCase {
            future_registry: future_registry,
        })) as &'static dyn ForCompletingExpiredTimers;
        let timer_manager = Box::leak(Box::new(KernelCell::new(TimerManager::new())));
        let ipc_manager = Box::leak(Box::new(KernelCell::new(IpcManager::new_with_notifier(notifier))));

        KernelServices {
            task_manager,
            future_registry,
            ipc_manager,
            timer_manager,
            scheduler: scheduler_cell,
            memory_manager: &MEMORY_MANAGER,
            timer_handler,
        }
    };

    #[cfg(not(test))]
    KERNEL_SERVICES.call_once(create_services);

    #[cfg(test)]
    {
        static TEST_INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        TEST_INIT.get_or_init(|| {
            KERNEL_SERVICES.call_once(create_services);
        });
    }
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

        let task = Task::new("test", 0x1000, 0);
        let handle = s.task_manager.borrow_mut().add_task(task).unwrap();
        assert_eq!(s.task_manager.borrow().get_state(handle), crate::task::TaskState::Created);

        let future = alloc::boxed::Box::new(crate::future::TaskCompletionFuture::new(handle));
        let fh = s.future_registry.borrow_mut().register(future);
        assert!(fh.is_some());
    }
}
