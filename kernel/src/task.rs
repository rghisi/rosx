use crate::cpu::Cpu;
use crate::kernel::{kernel};
use crate::task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt::{Display, Formatter};
use crate::elf::load_elf;
use crate::handle::FutureHandle;
use crate::task_stack::TaskStack;
use system::args::{MAX_ARGUMENT_BYTES, MAX_ARGUMENT_COUNT};
use system::error::ErrorCode;

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
    #[allow(dead_code)]
    stack: TaskStack,
    completion_future: Option<FutureHandle>,
}

impl Task {
    pub fn new<'a>(
        name: &'static str,
        entry_point: usize,
        entry_param: usize,
    ) -> SharedTask {
        let stack = TaskStack::new();
        let stack_pointer = stack.top();
        Box::new(Task {
            name,
            state: Created,
            yield_reason: None,
            stack_pointer,
            entry_point,
            entry_param,
            stack,
            completion_future: None,
        })
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

pub struct ElfTaskContext {
    elf_bytes: Vec<u8>,
    argument_strings: Vec<u8>,
    argument_pointers: Vec<*const u8>,
}

impl ElfTaskContext {
    fn new(elf_bytes: &[u8], arguments: &[&str]) -> Box<ElfTaskContext> {
        let mut argument_strings: Vec<u8> = Vec::new();
        for argument in arguments {
            argument_strings.extend_from_slice(argument.as_bytes());
            argument_strings.push(0);
        }
        ElfTaskContext::with_blob(elf_bytes, argument_strings)
    }

    fn with_blob(elf_bytes: &[u8], argument_strings: Vec<u8>) -> Box<ElfTaskContext> {
        let mut context = Box::new(ElfTaskContext {
            elf_bytes: elf_bytes.to_vec(),
            argument_strings,
            argument_pointers: Vec::new(),
        });
        context.install_argument_pointers();
        context
    }

    fn install_argument_pointers(&mut self) {
        let base = self.argument_strings.as_ptr();
        let mut start = 0;
        while start < self.argument_strings.len() {
            self.argument_pointers.push(unsafe { base.add(start) });
            let argument_len = self.argument_strings[start..]
                .iter()
                .position(|byte| *byte == 0)
                .unwrap();
            start += argument_len + 1;
        }
        self.argument_pointers.push(core::ptr::null());
    }

    fn argc(&self) -> i32 {
        self.argument_pointers.len() as i32 - 1
    }

    fn argv(&self) -> *const *const u8 {
        self.argument_pointers.as_ptr()
    }
}

pub fn validate_arguments_blob(blob: &[u8]) -> Result<(), ErrorCode> {
    if blob.len() > MAX_ARGUMENT_BYTES {
        return Err(ErrorCode::ElfArgumentLimitExceeded);
    }
    if blob.iter().filter(|byte| **byte == 0).count() > MAX_ARGUMENT_COUNT {
        return Err(ErrorCode::ElfArgumentLimitExceeded);
    }
    if blob.is_empty() || blob[blob.len() - 1] != 0 || blob[0] == 0 {
        return Err(ErrorCode::ElfArgumentBlobMalformed);
    }
    if blob.windows(2).any(|pair| pair[0] == 0 && pair[1] == 0) {
        return Err(ErrorCode::ElfArgumentBlobMalformed);
    }
    Ok(())
}

pub fn new_elf_task(elf: &[u8]) -> SharedTask {
    let context = ElfTaskContext::new(elf, &["ELF"]);
    Task::new("ELF", elf_task_wrapper as usize, Box::into_raw(context) as usize)
}

pub fn new_elf_task_with_arguments(elf: &[u8], arguments_blob: Vec<u8>) -> SharedTask {
    let context = ElfTaskContext::with_blob(elf, arguments_blob);
    Task::new("ELF", elf_task_wrapper as usize, Box::into_raw(context) as usize)
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
        assert_eq!(task.stack_pointer(), task.stack.top());
        assert!(task.stack.words().iter().all(|w| *w == 0));
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

    fn read_c_string(pointer: *const u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut index = 0;
        while unsafe { *pointer.add(index) } != 0 {
            bytes.push(unsafe { *pointer.add(index) });
            index += 1;
        }
        bytes
    }

    #[test]
    fn elf_task_context_stores_arguments_as_nul_terminated_strings() {
        let context = ElfTaskContext::new(&[], &["hello", "world"]);
        assert_eq!(context.argument_strings, b"hello\0world\0");
    }

    #[test]
    fn elf_task_context_without_arguments_holds_no_strings() {
        let context = ElfTaskContext::new(&[], &[]);
        assert!(context.argument_strings.is_empty());
        assert_eq!(context.argc(), 0);
        assert!(unsafe { context.argv().read().is_null() });
    }

    #[test]
    fn elf_task_context_argument_pointers_address_their_own_strings() {
        let context = ElfTaskContext::new(&[], &["hello", "", "x86_64"]);
        let argv = context.argv();
        assert_eq!(context.argc(), 3);
        assert_eq!(unsafe { argv.read() }, context.argument_strings.as_ptr());
        assert_eq!(read_c_string(unsafe { argv.add(0).read() }), b"hello");
        assert_eq!(read_c_string(unsafe { argv.add(1).read() }), b"");
        assert_eq!(read_c_string(unsafe { argv.add(2).read() }), b"x86_64");
        assert!(unsafe { argv.add(3).read().is_null() });
    }

    #[test]
    fn elf_task_context_keeps_strings_beyond_pointer_installation() {
        let context = ElfTaskContext::new(&[1, 2, 3], &["pi"]);
        let moved = context;
        assert_eq!(read_c_string(unsafe { moved.argv().read() }), b"pi");
        assert_eq!(moved.elf_bytes, &[1, 2, 3]);
    }

    #[test]
    fn with_blob_builds_context_with_three_arguments_and_null_terminator() {
        let context = ElfTaskContext::with_blob(&[], b"a\0b\0c\0".to_vec());
        assert_eq!(context.argc(), 3);
        let argv = context.argv();
        assert_eq!(read_c_string(unsafe { argv.add(0).read() }), b"a");
        assert_eq!(read_c_string(unsafe { argv.add(1).read() }), b"b");
        assert_eq!(read_c_string(unsafe { argv.add(2).read() }), b"c");
        assert!(unsafe { argv.add(3).read().is_null() });
    }

    #[test]
    fn validate_rejects_blob_without_trailing_nul_as_malformed() {
        assert_eq!(
            validate_arguments_blob(b"a\0b"),
            Err(ErrorCode::ElfArgumentBlobMalformed)
        );
    }

    #[test]
    fn validate_rejects_blob_with_double_nul_as_malformed() {
        assert_eq!(
            validate_arguments_blob(b"a\0\0b\0"),
            Err(ErrorCode::ElfArgumentBlobMalformed)
        );
    }

    #[test]
    fn validate_rejects_blob_with_leading_nul_as_malformed() {
        assert_eq!(
            validate_arguments_blob(b"\0a\0"),
            Err(ErrorCode::ElfArgumentBlobMalformed)
        );
    }

    #[test]
    fn validate_rejects_empty_blob_as_malformed() {
        assert_eq!(
            validate_arguments_blob(b""),
            Err(ErrorCode::ElfArgumentBlobMalformed)
        );
    }

    #[test]
    fn validate_accepts_canonical_three_argument_blob() {
        assert_eq!(validate_arguments_blob(b"a\0b\0c\0"), Ok(()));
    }

    #[test]
    fn validate_rejects_seventeen_arguments_as_limit_exceeded() {
        let blob = b"a\0".repeat(17);
        assert_eq!(
            validate_arguments_blob(&blob),
            Err(ErrorCode::ElfArgumentLimitExceeded)
        );
    }

    #[test]
    fn validate_rejects_blob_over_max_bytes_as_limit_exceeded() {
        let mut blob = vec![b'a'; MAX_ARGUMENT_BYTES + 1];
        blob.push(0);
        assert!(blob.len() > MAX_ARGUMENT_BYTES);
        assert_eq!(
            validate_arguments_blob(&blob),
            Err(ErrorCode::ElfArgumentLimitExceeded)
        );
    }

    #[test]
    fn validate_accepts_exactly_max_argument_count() {
        let blob = b"a\0".repeat(MAX_ARGUMENT_COUNT);
        assert_eq!(validate_arguments_blob(&blob), Ok(()));
    }

    #[test]
    fn validate_accepts_exactly_max_argument_bytes() {
        let mut blob = vec![b'a'; MAX_ARGUMENT_BYTES - 1];
        blob.push(0);
        assert_eq!(blob.len(), MAX_ARGUMENT_BYTES);
        assert_eq!(validate_arguments_blob(&blob), Ok(()));
    }

    #[test]
    fn new_elf_task_default_context_argv_remains_placeholder() {
        let task = new_elf_task(&[7]);
        assert_eq!(task.name(), "ELF");
        let context = unsafe { Box::from_raw(task.entry_param() as *mut ElfTaskContext) };
        assert_eq!(context.argc(), 1);
        assert_eq!(read_c_string(unsafe { context.argv().read() }), b"ELF");
        assert!(unsafe { context.argv().add(1).read().is_null() });
    }

    #[test]
    fn new_elf_task_with_arguments_wires_blob_into_task_context() {
        let task = new_elf_task_with_arguments(&[9], b"pi\0alpha\0".to_vec());
        let context = unsafe { Box::from_raw(task.entry_param() as *mut ElfTaskContext) };
        assert_eq!(context.argc(), 2);
        assert_eq!(read_c_string(unsafe { context.argv().read() }), b"pi");
        assert_eq!(
            read_c_string(unsafe { context.argv().add(1).read() }),
            b"alpha"
        );
    }

    #[test]
    fn new_and_with_blob_produce_equal_contexts_for_joined_arguments() {
        let arguments = ["alpha", "beta", "gamma"];
        let joined = ElfTaskContext::new(&[], &arguments);
        let mut blob = Vec::new();
        for argument in arguments {
            blob.extend_from_slice(argument.as_bytes());
            blob.push(0);
        }
        let moved = ElfTaskContext::with_blob(&[], blob);
        assert_eq!(joined.argument_strings, moved.argument_strings);
        assert_eq!(joined.argc(), moved.argc());
    }

    #[test]
    fn elf_task_context_owns_a_copy_of_the_image() {
        let mut source = vec![1u8, 2, 3, 4];
        let context = ElfTaskContext::with_blob(&source, Vec::new());
        source[0] = 9;
        assert_eq!(context.elf_bytes, vec![1, 2, 3, 4]);
    }
}

pub(crate) extern "C" fn elf_task_wrapper(context: usize) {
    let context = unsafe { Box::from_raw(context as *mut ElfTaskContext) };
    let image = load_elf(&context.elf_bytes, kernel().elf_arch).unwrap();
    let task_entry_point: extern "C" fn(i32, *const *const u8) -> i32 =
        unsafe { core::mem::transmute(image.entry) };

    kernel().execution_state.preemption_enabled = true;

    task_entry_point(context.argc(), context.argv());

    drop(image);
    drop(context);

    kernel().terminate_and_yield();
}