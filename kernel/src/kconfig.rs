use crate::platform::cpu::Cpu;
use crate::platform::elf::ElfArch;
use crate::platform::portio::PortIo;
use crate::scheduler::SchedulerFactory;

pub struct KConfig {
    pub cpu: &'static dyn Cpu,
    pub elf_arch: &'static dyn ElfArch,
    pub port_io: &'static dyn PortIo,
    pub scheduler_factory: SchedulerFactory,
}

unsafe impl Sync for KConfig {}

