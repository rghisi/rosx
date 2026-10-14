use crate::cpu::Cpu;
use crate::kernel::{kernel};
use crate::task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use alloc::boxed::Box;
use core::fmt::{Display, Formatter};
use core::mem::{size_of, MaybeUninit};
use core::ptr::{addr_of_mut, write, write_bytes};
use crate::elf::load_elf;
use crate::handle::FutureHandle;

const STACK_WORDS: usize = 2048;
type TaskStack = [usize; STACK_WORDS];

pub use crate::handle::TaskHandle;
pub type SharedTask = Box<Task>;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum YieldReason {
    Voluntary,
    Preempted,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TaskState {
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
    stack: TaskStack,
    completion_future: Option<FutureHandle>,
}

impl Task {
    pub fn new<'a>(
        name: &'static str,
        entry_point: usize,
        entry_param: usize,
    ) -> SharedTask {
        let mut task: Box<MaybeUninit<Task>> = Box::new_uninit();
        let task_ptr: *mut Task = Box::as_mut_ptr(&mut task).cast();
        // Safety: every Task field is written exactly once below before assume_init; all fields are Copy with no Drop impl.
        unsafe {
            let stack_ptr = addr_of_mut!((*task_ptr).stack);
            let stack_pointer = stack_ptr.addr() + size_of::<TaskStack>();
            write(addr_of_mut!((*task_ptr).name), name);
            write(addr_of_mut!((*task_ptr).state), Created);
            write(addr_of_mut!((*task_ptr).yield_reason), None);
            write(addr_of_mut!((*task_ptr).stack_pointer), stack_pointer);
            write(addr_of_mut!((*task_ptr).entry_point), entry_point);
            write(addr_of_mut!((*task_ptr).entry_param), entry_param);
            write_bytes(stack_ptr.cast::<u8>(), 0, size_of::<TaskStack>());
            write(addr_of_mut!((*task_ptr).completion_future), None);
            task.assume_init()
        }
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

    pub fn prepare_entry(&mut self, cpu: &dyn Cpu) {
        let new_stack_pointer = cpu.initialize_stack(
            self.stack_pointer(),
            self.entry_point(),
            self.entry_param(),
            0,
        );
        self.set_stack_pointer(new_stack_pointer);
        self.set_ready();
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

    kernel().execution_state.preemption_enabled = true;

    task_entry_point();

    kernel().terminate_and_yield();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::arena::GenerationalArena;

    fn make_future_handle() -> FutureHandle {
        let mut arena: GenerationalArena<u8, 1> = GenerationalArena::new();
        FutureHandle::from(arena.add(0u8).unwrap())
    }

    #[test]
    fn task_new_field_integrity() {
        let task = Task::new("t", 0x1111, 0x2222);
        assert_eq!(task.name(), "t");
        assert_eq!(task.entry_point(), 0x1111);
        assert_eq!(task.entry_param(), 0x2222);
        assert_eq!(task.state(), Created);
        assert!(task.yield_reason().is_none());
        assert!(task.completion_future().is_none());
        assert_ne!(task.stack_pointer(), 0);
        assert_eq!(
            task.stack_pointer(),
            task.stack.as_ptr().addr() + core::mem::size_of_val(&task.stack),
        );
        assert!(task.stack.iter().all(|w| *w == 0));
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