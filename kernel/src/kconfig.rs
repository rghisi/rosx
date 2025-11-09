use crate::cpu::Cpu;
use crate::task::SharedTask;

pub struct KConfig {
    pub cpu: &'static dyn Cpu,
    pub idle_task_factory: fn() -> SharedTask,
}

unsafe impl Sync for KConfig {}
