use crate::cpu::Cpu;
use crate::elf::ElfArch;
use crate::scheduler::SchedulerFactory;

pub struct KConfig {
    pub cpu: &'static dyn Cpu,
    pub elf_arch: &'static dyn ElfArch,
    pub scheduler_factory: SchedulerFactory,
}

unsafe impl Sync for KConfig {}

