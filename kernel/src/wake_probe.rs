use crate::kernel::kernel;
use crate::kernel_services::services;
use crate::task::TaskHandle;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

#[cfg(not(test))]
macro_rules! probe_out {
    ($($arg:tt)*) => {};
}

#[cfg(test)]
macro_rules! probe_out {
    ($($arg:tt)*) => {};
}

struct BusyCell {
    depth: AtomicUsize,
    holder_if: AtomicUsize,
}

impl BusyCell {
    const fn new() -> Self {
        Self {
            depth: AtomicUsize::new(0),
            holder_if: AtomicUsize::new(0),
        }
    }

    fn enter(&'static self, cell: &'static str, op: &str, ctx: &str) {
        let now_if = ifflag() as usize;
        let was = self.depth.fetch_add(1, Relaxed);
        if was == 0 {
            self.holder_if.store(now_if, Relaxed);
        } else if now_if == 0 && self.holder_if.load(Relaxed) == 1 {
            reenter(cell, op, ctx);
        }
    }

    fn exit(&'static self) {
        if self.depth.fetch_sub(1, Relaxed) == 1 {
            self.holder_if.store(0, Relaxed);
        }
    }
}

fn ifflag() -> u8 {
    #[cfg(test)]
    {
        0
    }
    #[cfg(not(test))]
    {
        if kernel().interrupts_enabled() {
            1
        } else {
            0
        }
    }
}

static SCHED: BusyCell = BusyCell::new();
static FR: BusyCell = BusyCell::new();
static MM: BusyCell = BusyCell::new();
static REENTER_BUDGET: AtomicU32 = AtomicU32::new(12);
static PENDING_WAKE: [AtomicUsize; 256] = [const { AtomicUsize::new(0) }; 256];

pub(crate) fn sched_enter(op: &str, ctx: &str) {
    SCHED.enter("scheduler", op, ctx);
}
pub(crate) fn sched_exit() {
    SCHED.exit();
}
pub(crate) fn fr_enter(op: &str, ctx: &str) {
    FR.enter("future_registry", op, ctx);
}
pub(crate) fn fr_exit() {
    FR.exit();
}
pub(crate) fn mm_enter(op: &str, ctx: &str) {
    MM.enter("mailbox_manager", op, ctx);
}
pub(crate) fn mm_exit() {
    MM.exit();
}

fn reenter(cell: &'static str, op: &str, ctx: &str) {
    if REENTER_BUDGET.fetch_sub(1, Relaxed) > 0 {
        probe_out!(
            "[PROBE] REENTER cell={} op={} ctx={} if={}",
            cell,
            op,
            ctx,
            ifflag()
        );
    }
}

fn name_of(handle: TaskHandle) -> &'static str {
    services()
        .task_manager
        .borrow_mut()
        .borrow_task_mut(handle)
        .map(|task| task.name())
        .unwrap_or("?")
}

pub(crate) fn park(task: &str, fh: usize) {
    probe_out!("[PROBE] park task={} fh={} if={}", task, fh, ifflag());
}

pub(crate) fn notify(fh: usize, waiters: usize) {
    if waiters > 0 {
        probe_out!("[PROBE] notify fh={} waiters={} if={}", fh, waiters, ifflag());
    }
}

pub(crate) fn wake(handle: TaskHandle) {
    PENDING_WAKE[handle.index() as usize % 256].store(1, Relaxed);
    probe_out!(
        "[PROBE] wake task={} hidx={} if={}",
        name_of(handle),
        handle.index(),
        ifflag()
    );
}

pub(crate) fn pushed(handle: TaskHandle) {
    probe_out!(
        "[PROBE] pushed task={} hidx={} if={}",
        name_of(handle),
        handle.index(),
        ifflag()
    );
}

pub(crate) fn picked(handle: TaskHandle) {
    if PENDING_WAKE[handle.index() as usize % 256].load(Relaxed) == 1 {
        probe_out!(
            "[PROBE] picked task={} hidx={} if={}",
            name_of(handle),
            handle.index(),
            ifflag()
        );
    }
}

pub(crate) fn swapped_in(handle: TaskHandle) {
    if PENDING_WAKE[handle.index() as usize % 256].swap(0, Relaxed) == 1 {
        probe_out!(
            "[PROBE] swapped-in task={} hidx={} if={}",
            name_of(handle),
            handle.index(),
            ifflag()
        );
    }
}
