use alloc::boxed::Box;
use alloc::vec::Vec;
use system::future::{Future, FutureHandle};
use system::ipc::IpcMessage;
use crate::messages::HardwareInterrupt;
use crate::task::{TaskHandle, TaskState, YieldReason};

pub(crate) trait ForWakingTasks: Send + Sync {
    fn wake_tasks(&self, handles: Vec<TaskHandle>);
}

pub(crate) trait ForNotifyingFutures: Send + Sync {
    fn register(&self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle>;
    fn notify(&self, handle: FutureHandle);
    fn complete_ipc_message(&self, handle: FutureHandle, message: IpcMessage);
}

pub trait ForCompletingExpiredTimers: Send + Sync {
    fn complete_timer_future(&self, handle: FutureHandle);
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SwitchOutcome {
    Yielded(TaskHandle, YieldReason),
    Blocked(TaskHandle),
    Terminated(TaskHandle),
    Unchanged(TaskHandle),
}

pub trait ForSwitchingTaskContext: Send + Sync {
    fn switch_to_task(&self, handle: TaskHandle) -> SwitchOutcome;
}

pub trait ForReadingSystemTime: Send + Sync {
    fn now(&self) -> u64;
}

pub trait ForExpiringTimers: Send + Sync {
    fn pop_expired(&self, now: u64) -> Option<Vec<FutureHandle>>;
}

pub trait ForHandlingHardwareInterrupts: Send + Sync {
    fn handle(&self, interrupt: HardwareInterrupt);
}

pub trait ForManagingTasks: Send + Sync {
    fn get_state(&self, handle: TaskHandle) -> TaskState;
    fn set_state(&self, handle: TaskHandle, state: TaskState);
    fn remove_task(&self, handle: TaskHandle);
}
