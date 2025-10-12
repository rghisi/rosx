use cpu::Cpu;
use main_thread::MainThread;
use task::{SharedTask, Task};

pub(crate) static mut MAIN_THREAD_PTR: Option<*mut MainThread> = None;
pub(crate) static mut MAIN_THREAD_TASK_PTR: Option<*mut Task> = None;
pub(crate) static mut CURRENT_TASK: Option<SharedTask> = None;
pub(crate) static mut CPU_PTR: Option<&'static dyn Cpu> = None;