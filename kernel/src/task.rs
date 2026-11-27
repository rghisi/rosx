use crate::platform::cpu::Cpu;
use crate::kernel::{kernel};
use crate::task::TaskState::{Blocked, Created, Ready, Running, Terminated};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt::{Display, Formatter};
use crate::elf::{inspect, load_into, ElfError};
use crate::handle::FutureHandle;
use crate::memory::memory_manager::MEMORY_MANAGER;
use crate::memory::program_memory::ProgramMemory;
use crate::platform::elf::ElfArch;
use crate::task_stack::{StackObservation, TaskStack, STACK_GUARD_BYTES};
use core::sync::atomic::{AtomicUsize, Ordering};
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
pub const TASK_NAME_BYTES: usize = 32;

pub struct TaskName {
    bytes: [u8; TASK_NAME_BYTES],
    length: usize,
}

impl TaskName {
    fn new(name: &str) -> TaskName {
        let mut length = name.len().min(TASK_NAME_BYTES);
        while length > 0 && !name.is_char_boundary(length) {
            length -= 1;
        }
        let mut bytes = [0u8; TASK_NAME_BYTES];
        bytes[..length].copy_from_slice(&name.as_bytes()[..length]);
        TaskName { bytes, length }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length]).unwrap()
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct GroupId(usize);

impl GroupId {
    pub(crate) fn allocate() -> GroupId {
        static NEXT_GROUP_ID: AtomicUsize = AtomicUsize::new(1);
        GroupId(NEXT_GROUP_ID.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn value(&self) -> usize {
        self.0
    }
}

pub struct Task {
    name: TaskName,
    state: TaskState,
    yield_reason: Option<YieldReason>,
    stack_pointer: usize,
    entry_point: usize,
    entry_param: usize,
    stack: TaskStack,
    completion_future: Option<FutureHandle>,
    group: GroupId,
}

impl Task {
    pub fn new(
        name: &str,
        entry_point: usize,
        entry_param: usize,
    ) -> SharedTask {
        Task::in_group(name, entry_point, entry_param, GroupId::allocate())
    }

    fn in_group(
        name: &str,
        entry_point: usize,
        entry_param: usize,
        group: GroupId,
    ) -> SharedTask {
        let stack = TaskStack::new(group);
        let stack_pointer = stack.top();
        Box::new(Task {
            name: TaskName::new(name),
            state: Created,
            yield_reason: None,
            stack_pointer,
            entry_point,
            entry_param,
            stack,
            completion_future: None,
            group,
        })
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
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

    pub(crate) fn stack_base(&self) -> usize {
        self.stack.base()
    }

    pub(crate) fn observe_stack(&mut self) -> StackObservation {
        self.stack.observe_depth();
        if self.stack.is_exhausted() {
            StackObservation::Exhausted
        } else if self.stack_pointer < self.stack.base() + STACK_GUARD_BYTES {
            StackObservation::SpillIntoGuard
        } else {
            StackObservation::WithinBudget
        }
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
    pub(crate) fn group(&self) -> GroupId {
        self.group
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
        Task::new(name, task_wrapper as *const () as usize, job as usize)
    }
}

pub fn idle_task_factory(cpu: &'static dyn Cpu) -> Box<Task> {
    let cpu_ptr = Box::into_raw(Box::new(cpu)) as usize;
    Task::new("[K] Idle", idle_entry as *const () as usize, cpu_ptr)
}

extern "C" fn idle_entry(cpu_ptr: usize) {
    kernel().execution_state.preemption_enabled = true;
    let cpu: &'static dyn Cpu = unsafe { *(cpu_ptr as *const &'static dyn Cpu) };
    loop {
        cpu.enable_interrupts();
        cpu.halt();
    }
}

pub fn new_entrypoint_task(entrypoint: usize, group: GroupId) -> SharedTask {
    Task::in_group("EPT", task_wrapper as *const () as usize, entrypoint, group)
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

pub fn first_argument(blob: &[u8]) -> Option<&str> {
    let end = blob.iter().position(|byte| *byte == 0)?;
    core::str::from_utf8(&blob[..end]).ok()
}

#[repr(C)]
struct ElfProgram {
    entry: usize,
    argv: *const *const u8,
    argc: i32,
}

pub fn new_elf_task_with_arguments(
    name: &str,
    elf: &[u8],
    arguments_blob: Vec<u8>,
    elf_arch: &dyn ElfArch,
) -> Result<SharedTask, ElfError> {
    let layout = inspect(elf)?;
    let group = GroupId::allocate();
    let argc = arguments_blob.iter().filter(|byte| **byte == 0).count();
    let record_at = (layout.span + core::mem::align_of::<usize>() - 1)
        & !(core::mem::align_of::<usize>() - 1);
    let table_at = record_at + core::mem::size_of::<ElfProgram>();
    let strings_at = table_at + (argc + 1) * core::mem::size_of::<usize>();
    let total_len = strings_at + arguments_blob.len();

    let mut block = ProgramMemory::allocate(group, total_len, layout.align)
        .ok_or(ElfError::OutOfMemory)?;
    let image_len = layout.span;
    let entry = match load_into(elf, &mut block.bytes_mut()[..image_len], &layout, elf_arch) {
        Ok(entry) => entry,
        Err(error) => {
            unsafe { MEMORY_MANAGER.release_group_allocations(group.value()) };
            return Err(error);
        }
    };

    let base = block.base();
    unsafe {
        let strings = base + strings_at;
        core::ptr::copy_nonoverlapping(
            arguments_blob.as_ptr(),
            strings as *mut u8,
            arguments_blob.len(),
        );
        let table = (base + table_at) as *mut usize;
        let mut start = 0;
        let mut index = 0;
        while start < arguments_blob.len() {
            table.add(index).write(strings + start);
            let argument_len = arguments_blob[start..]
                .iter()
                .position(|byte| *byte == 0)
                .unwrap();
            start += argument_len + 1;
            index += 1;
        }
        table.add(argc).write(0);
        core::ptr::write(
            (base + record_at) as *mut ElfProgram,
            ElfProgram {
                entry,
                argv: table as *const *const u8,
                argc: argc as i32,
            },
        );
    }

    Ok(Task::in_group(
        name,
        elf_task_wrapper as *const () as usize,
        base + record_at,
        group,
    ))
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
    use crate::elf::tests::{
        elf64_file_larger_than_span_fixture, elf64_with_unreachable_reloc_fixture,
        minimal_elf64_fixture,
    };
    use crate::task_stack::STACK_WATERMARK;

    fn make_future_handle() -> FutureHandle {
        let mut arena: GenerationalArena<u8> = GenerationalArena::new(1);
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
        assert!(task.stack.words().iter().all(|w| *w == STACK_WATERMARK));
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

    struct StubElfArch;

    impl ElfArch for StubElfArch {
        fn apply_relocation(&self, _base: usize, _offset: usize, _info: u64, _addend: i64) {}
    }

    fn program_record(task: &Task) -> ElfProgram {
        unsafe { core::ptr::read_unaligned(task.entry_param() as *const ElfProgram) }
    }

    fn argv_strings(record: ElfProgram) -> Vec<Vec<u8>> {
        let mut arguments = Vec::new();
        let mut index = 0;
        while !unsafe { record.argv.add(index).read() }.is_null() {
            arguments.push(read_c_string(unsafe { record.argv.add(index).read() }));
            index += 1;
        }
        arguments
    }

    fn heap_used() -> usize {
        let stats = MEMORY_MANAGER.stats();
        stats.total_bytes - stats.free_bytes
    }

    #[test]
    fn new_elf_task_writes_argv_into_the_program_record() {
        let elf = minimal_elf64_fixture(42);
        let task =
            new_elf_task_with_arguments("pi", &elf, b"pi\0alpha\0".to_vec(), &StubElfArch).unwrap();
        assert_eq!(task.name(), "pi");
        let record = program_record(&task);
        assert_eq!(record.argc, 2);
        assert_eq!(argv_strings(record), vec![b"pi".to_vec(), b"alpha".to_vec()]);
    }

    #[test]
    fn the_record_argv_addresses_the_groups_copy_not_the_callers_blob() {
        let elf = minimal_elf64_fixture(0);
        let mut blob = b"pi\0".to_vec();
        let task =
            new_elf_task_with_arguments("pi", &elf, blob.clone(), &StubElfArch).unwrap();
        blob[0] = b'x';
        assert_eq!(argv_strings(program_record(&task)), vec![b"pi".to_vec()]);
    }

    #[test]
    fn a_task_constructed_without_arguments_reports_an_empty_argv() {
        let elf = minimal_elf64_fixture(0);
        let task =
            new_elf_task_with_arguments("pi", &elf, Vec::new(), &StubElfArch).unwrap();
        let record = program_record(&task);
        assert_eq!(record.argc, 0);
        assert!(unsafe { record.argv.read() }.is_null());
    }

    #[test]
    fn the_record_argv_addresses_arguments_including_an_empty_string() {
        let elf = minimal_elf64_fixture(0);
        let task = new_elf_task_with_arguments(
            "app",
            &elf,
            b"hello\0\0x86_64\0".to_vec(),
            &StubElfArch,
        )
        .unwrap();
        let record = program_record(&task);
        assert_eq!(record.argc, 3);
        assert_eq!(
            argv_strings(record),
            vec![b"hello".to_vec(), b"".to_vec(), b"x86_64".to_vec()]
        );
    }

    #[test]
    fn with_three_arguments_the_record_table_ends_with_null() {
        let elf = minimal_elf64_fixture(0);
        let task =
            new_elf_task_with_arguments("a", &elf, b"a\0b\0c\0".to_vec(), &StubElfArch).unwrap();
        let record = program_record(&task);
        assert_eq!(record.argc, 3);
        assert_eq!(
            argv_strings(record),
            vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]
        );
    }

    #[test]
    fn a_loaded_image_is_owned_by_the_tasks_group() {
        let elf = minimal_elf64_fixture(0);
        let task =
            new_elf_task_with_arguments("app", &elf, b"app\0".to_vec(), &StubElfArch).unwrap();
        let group_bytes = MEMORY_MANAGER.group_bytes(task.group().value());
        assert!(
            group_bytes >= elf.len(),
            "the group's stamped bytes must include the loaded image span"
        );
    }

    #[test]
    fn a_load_failure_after_allocation_releases_the_group_block() {
        let elf = elf64_with_unreachable_reloc_fixture();
        let before = heap_used();
        let result = new_elf_task_with_arguments("app", &elf, b"app\0".to_vec(), &StubElfArch);
        assert!(matches!(result, Err(ElfError::MalformedSegment)));
        assert_eq!(heap_used(), before, "a rejected program leaves no stamped block");
    }

    #[test]
    fn the_group_holds_the_image_not_a_copy_of_the_elf_file() {
        let elf = elf64_file_larger_than_span_fixture(0);
        let task =
            new_elf_task_with_arguments("app", &elf, b"app\0".to_vec(), &StubElfArch).unwrap();
        let stack_bytes = crate::task_stack::STACK_WORDS * core::mem::size_of::<usize>();
        let retained = MEMORY_MANAGER.group_bytes(task.group().value()) - stack_bytes;
        assert!(
            retained < elf.len(),
            "the retained program memory is the image plus argv, smaller than the ELF file"
        );
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
    fn first_argument_yields_the_blobs_leading_string() {
        assert_eq!(first_argument(b"vfsd\0"), Some("vfsd"));
        assert_eq!(first_argument(b"pi\0alpha\0"), Some("pi"));
    }

    #[test]
    fn first_argument_without_a_terminator_is_none() {
        assert_eq!(first_argument(b"pi"), None);
        assert_eq!(first_argument(b""), None);
    }

    #[test]
    fn first_argument_rejects_a_non_utf8_name() {
        assert_eq!(first_argument(&[0xff, 0x00]), None);
    }

    #[test]
    fn elf_task_takes_its_name_from_the_argument_blob() {
        let elf = minimal_elf64_fixture(0);
        let name = first_argument(b"vfsd\0").unwrap();
        let task =
            new_elf_task_with_arguments(name, &elf, b"vfsd\0".to_vec(), &StubElfArch).unwrap();
        assert_eq!(task.name(), "vfsd");
        let record = program_record(&task);
        assert_eq!(record.argc, 1);
        assert_eq!(argv_strings(record), vec![b"vfsd".to_vec()]);
    }

    #[test]
    fn task_name_truncates_at_the_capacity() {
        let name = "x".repeat(TASK_NAME_BYTES + 8);
        let task = Task::new(&name, 0, 0);
        assert_eq!(task.name(), "x".repeat(TASK_NAME_BYTES));
    }

    #[test]
    fn task_name_truncation_stays_on_a_char_boundary() {
        let name = "a".repeat(TASK_NAME_BYTES - 1) + "é";
        let task = Task::new(&name, 0, 0);
        assert_eq!(task.name(), "a".repeat(TASK_NAME_BYTES - 1));
    }

    #[test]
    fn freshly_loaded_images_get_distinct_groups() {
        let first = Task::new("app", 0, 0);
        let second = Task::new("app", 0, 0);
        assert_ne!(first.group(), second.group());
    }

    #[test]
    fn entrypoint_task_carries_the_caller_group() {
        let caller = Task::new("app", 0, 0);
        let spawned = new_entrypoint_task(0x1000, caller.group());
        assert_eq!(spawned.group(), caller.group());
    }
}

pub(crate) extern "C" fn elf_task_wrapper(record: usize) {
    let program = unsafe { core::ptr::read_unaligned(record as *const ElfProgram) };
    let task_entry_point: extern "C" fn(i32, *const *const u8) -> i32 =
        unsafe { core::mem::transmute(program.entry) };

    kernel().execution_state.preemption_enabled = true;

    let return_code = task_entry_point(program.argc, program.argv);

    kernel().terminate_with_status(system::exit::encode(system::exit::EXIT_REASON_CLEAN, return_code as u32));
}