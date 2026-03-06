use collections::generational_arena::Handle;
use crate::cpu::Cpu;
use crate::kernel::{kernel};
use crate::task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use alloc::boxed::Box;
use core::fmt::{Display, Formatter};
use crate::elf::load_elf;
use system::future::FutureHandle;

pub(crate) type TaskHandle = Handle;
pub type SharedTask = Box<Task>;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum YieldReason {
    Voluntary,
    Preempted,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum TaskState {
    Created,
    Ready,
    Running,
    Blocked,
    Terminated,
}

impl Display for TaskState {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Created => {
                write!(f, "Created")
            }
            Ready => {
                write!(f, "Ready")
            }
            Running => {
                write!(f, "Running")
            }
            TaskState::Blocked => {
                write!(f, "Blocked")
            }
            Terminated => {
                write!(f, "Terminated")
            }
        }
    }
}
pub struct Task {
    name: &'static str,
    state: TaskState,
    yield_reason: Option<YieldReason>,
    stack_pointer: usize,
    entry_point: usize,
    entry_param: usize,
    stack: [usize; 2048], //16KB on 64bit systems
    completion_future: Option<FutureHandle>,
}

impl Task {
    pub fn new<'a>(
        name: &'static str,
        entry_point: usize,
        entry_param: usize,
    ) -> SharedTask {
        let mut task = Box::new(Task {
            name,
            state: Created,
            yield_reason: None,
            stack_pointer: 0,
            entry_point,
            entry_param,
            stack: [0; 2048],
            completion_future: None,
        });

        unsafe {
            let stack_pointer = task.stack.as_mut_ptr().add(task.stack.len()).addr();
            task.set_stack_pointer(stack_pointer);
        }

        task
    }
    pub fn name(&self) -> &'static str {
        self.name
    }
    pub fn stack_pointer(&self) -> usize {
        self.stack_pointer
    }
    pub fn stack_pointer_mut(&mut self) -> *mut usize {
        &mut self.stack_pointer as *mut usize
    }
    pub fn set_stack_pointer(&mut self, new_stack_pointer: usize) {
        self.stack_pointer = new_stack_pointer;
    }
    pub(crate) fn state(&self) -> TaskState {
        self.state
    }

    pub(crate) fn set_state(&mut self, new_state: TaskState) {
        self.state = new_state;
    }
    pub fn set_ready(&mut self) {
        self.state = Ready;
    }
    pub fn set_running(&mut self) {
        self.state = Running;
    }
    pub fn set_terminated(&mut self) {
        self.state = Terminated;
    }

    pub fn set_blocked(&mut self) {
        self.state = Blocked;
    }
    pub fn yield_reason(&self) -> Option<YieldReason> {
        self.yield_reason
    }
    pub fn set_yield_reason(&mut self, reason: YieldReason) {
        self.yield_reason = Some(reason);
    }
    pub fn is_schedulable(&self) -> bool {
        self.state != Created && self.state != Terminated
    }

    pub(crate) fn completion_future(&self) -> Option<FutureHandle> {
        self.completion_future
    }

    pub(crate) fn set_completion_future(&mut self, handle: FutureHandle) {
        self.completion_future = Some(handle);
    }

    pub fn entry_point(&self) -> usize {
        self.entry_point
    }
    pub fn entry_param(&self) -> usize {
        self.entry_param
    }
}

#[derive(Copy, Clone, Debug)]
pub struct FunctionTask {}

impl FunctionTask {
    pub fn new(name: &'static str, job: fn()) -> SharedTask {
        Task::new(name, task_wrapper as usize, job as usize)
    }
}

pub fn idle_task_factory(cpu: &'static dyn Cpu) -> Box<Task> {
    let cpu_ptr = Box::into_raw(Box::new(cpu)) as usize;
    Task::new("[K] Idle", idle_entry as usize, cpu_ptr)
}

extern "C" fn idle_entry(cpu_ptr: usize) {
    kernel().execution_state.preemption_enabled = true;
    let cpu: &'static dyn Cpu = unsafe { *(cpu_ptr as *const &'static dyn Cpu) };
    loop {
        cpu.halt();
    }
}

pub fn new_entrypoint_task(entrypoint: usize) -> SharedTask {
    Task::new("EPT", task_wrapper as usize, entrypoint)
}

pub fn new_elf_task(elf: &'static [u8]) -> SharedTask {
    let elf_ptr = Box::into_raw(Box::new(elf)) as usize;
    Task::new("ELF", elf_task_wrapper as usize, elf_ptr)
}

pub(crate) extern "C" fn task_wrapper(entry_point: usize) {
    let task_entry_point: fn() = unsafe { core::mem::transmute(entry_point) };

    crate::kprintln!("[TASK] wrapper entered");
    kernel().execution_state.preemption_enabled = true;

    let probe = Box::new(0xDEADBEEFu32);
    crate::kprintln!("[TASK] alloc probe: {:#x}", *probe);
    drop(probe);

    crate::kprintln!("[TASK] issuing direct int 0x80");
    let result: usize;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("eax") 0usize => result,
            in("ebx") 0usize,
            in("ecx") 0usize,
            in("edx") 0usize,
            options(nostack),
        );
    }
    crate::kprintln!("[TASK] after int 0x80, result={}", result);

    task_entry_point();

    kernel().terminate_and_yield();
}

#[cfg(test)]
mod tests {
    use super::*;
    use collections::generational_arena::GenerationalArena;

    fn make_future_handle() -> FutureHandle {
        let mut arena: GenerationalArena<u8, 1> = GenerationalArena::new();
        arena.add(0u8).unwrap()
    }

    #[test]
    fn new_task_has_no_completion_future() {
        let task = Task::new("test", 0, 0);
        assert!(task.completion_future().is_none());
    }

    #[test]
    fn can_set_and_get_completion_future() {
        let mut task = Task::new("test", 0, 0);
        let fh = make_future_handle();
        task.set_completion_future(fh);
        assert_eq!(task.completion_future(), Some(fh));
    }
}

pub(crate) extern "C" fn elf_task_wrapper(elf: usize) {
    let elf_bytes: &[u8] = unsafe { *Box::from_raw(elf as *mut &[u8]) };
    let image = load_elf(elf_bytes, kernel().elf_arch).unwrap();
    let task_entry_point: fn() = unsafe { core::mem::transmute(image.entry) };

    kernel().execution_state.preemption_enabled = true;

    task_entry_point();

    drop(image);

    kernel().terminate_and_yield();
}