use cpu::Cpu;
use task_scheduler_round_robin::RoundRobin;
use task::{SharedTask, Task};
use task_scheduler::TaskScheduler;

pub(crate) static mut MAIN_THREAD_PTR: Option<*mut dyn TaskScheduler> = None;
pub(crate) static mut MAIN_THREAD_TASK_PTR: Option<*mut Task> = None;
pub(crate) static mut CURRENT_TASK: Option<SharedTask> = None;
pub(crate) static mut CPU_PTR: Option<&'static dyn Cpu> = None;