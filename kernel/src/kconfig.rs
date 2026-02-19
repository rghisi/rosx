use crate::cpu::Cpu;
use crate::scheduler::SchedulerFactory;

pub struct KConfig {
    pub cpu: &'static dyn Cpu,
    pub scheduler_factory: SchedulerFactory,
}

unsafe impl Sync for KConfig {}

