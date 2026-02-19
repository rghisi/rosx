use crate::messages::HardwareInterrupt;
use crate::task::TaskHandle;
use alloc::boxed::Box;
use system::future::FutureHandle;

pub trait Scheduler {
    fn run(&mut self);
    fn push_task(&mut self, handle: TaskHandle);
    fn push_blocked(&mut self, task_handle: TaskHandle, future_handle: FutureHandle);
    fn push_hardware_interrupt(&mut self, interrupt: HardwareInterrupt);
    fn set_idle_task(&mut self, handle: TaskHandle) -> Result<(), ()>;
}

pub type SchedulerFactory = fn() -> Box<dyn Scheduler>;