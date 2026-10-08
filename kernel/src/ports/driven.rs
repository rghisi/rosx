use alloc::boxed::Box;
use alloc::vec::Vec;
use system::future::{Future, FutureHandle};
use system::ipc::IpcMessage;
use crate::messages::HardwareInterrupt;
use crate::task::{TaskHandle, TaskState, YieldReason};

/// Port: wake blocked tasks when a future resolves.
pub(crate) trait ForWakingTasks: Send + Sync {
    fn wake_tasks(&self, handles: Vec<TaskHandle>);
}

/// Port: register futures, notify them, complete IPC messages.
pub(crate) trait ForNotifyingFutures: Send + Sync {
    fn register(&self, future: Box<dyn Future + Send + Sync>) -> Option<FutureHandle>;
    fn notify(&self, handle: FutureHandle);
    fn complete_ipc_message(&self, handle: FutureHandle, message: IpcMessage);
}

/// Port: complete and notify a timer-expired future.
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

/// Port: read the current system time.
pub trait ForReadingSystemTime: Send + Sync {
    fn now(&self) -> u64;
}

/// Port: pop the timer futures that have expired at `now`.
pub trait ForExpiringTimers: Send + Sync {
    fn pop_expired(&self, now: u64) -> Option<Vec<FutureHandle>>;
}

/// Port: deliver a hardware interrupt to its subsystem (e.g. keyboard).
pub trait ForHandlingHardwareInterrupts: Send + Sync {
    fn handle(&self, interrupt: HardwareInterrupt);
}

/// Port: manage task states and lifecycle.
pub trait ForManagingTasks: Send + Sync {
    fn get_state(&self, handle: TaskHandle) -> TaskState;
    fn set_state(&self, handle: TaskHandle, state: TaskState);
    fn remove_task(&self, handle: TaskHandle);
}
