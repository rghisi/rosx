#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), feature(alloc_error_handler))]

extern crate alloc;
extern crate collections;
extern crate lazy_static;
extern crate system;

use alloc::boxed::Box;
use alloc::vec::Vec;
use system::future::{Future, FutureHandle};
use system::ipc::IpcMessage;
use crate::task::{TaskHandle, YieldReason};

pub mod cpu;
pub mod default_output;
pub mod elf;
pub mod future;
pub mod ipc;
pub mod kconfig;
pub mod kernel;
pub(crate) mod kernel_cell;
pub(crate) mod kernel_services;
mod keyboard;
pub mod memory;
pub mod messages;
pub mod once;
pub mod panic;
pub mod scheduler;
pub(crate) mod state;
pub mod syscall;
pub mod task;
pub(crate) mod task_manager;

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
