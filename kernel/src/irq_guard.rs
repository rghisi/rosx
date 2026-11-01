use crate::kernel_cell::KernelCell;
use crate::platform::cpu::Cpu;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

pub(crate) struct IrqCell {
    name: &'static str,
    depth: AtomicU32,
    trip_budget: AtomicU32,
}

pub(crate) struct IrqGuard<'a> {
    cell: &'a IrqCell,
    saved_if: bool,
}

impl IrqCell {
    pub(crate) const fn new(name: &'static str) -> Self {
        Self {
            name,
            depth: AtomicU32::new(0),
            trip_budget: AtomicU32::new(3),
        }
    }

    pub(crate) fn acquire(&'static self) -> IrqGuard<'static> {
        let prior_depth = self.depth.fetch_add(1, Relaxed);
        if prior_depth > 0 {
            self.trip();
        }
        let cpu = *GUARD_CPU.borrow();
        let saved_if = match cpu {
            Some(cpu) => {
                let was_enabled = cpu.are_interrupts_enabled();
                cpu.disable_interrupts();
                was_enabled
            }
            None => false,
        };
        IrqGuard {
            cell: self,
            saved_if,
        }
    }

    fn trip(&self) {
        let budget = self.trip_budget.load(Relaxed);
        if budget > 0
            && self
                .trip_budget
                .compare_exchange(budget, budget - 1, Relaxed, Relaxed)
                .is_ok()
        {
            print_trip(self.name);
        }
    }
}

#[cfg(not(test))]
fn print_trip(name: &'static str) {
    crate::kprintln!("[IRQGUARD] re-entrant section: {}", name);
}

#[cfg(test)]
fn print_trip(_name: &'static str) {}

impl Drop for IrqGuard<'_> {
    fn drop(&mut self) {
        self.cell.depth.fetch_sub(1, Relaxed);
        if self.saved_if {
            if let Some(cpu) = *GUARD_CPU.borrow() {
                cpu.enable_interrupts();
            }
        }
    }
}

static GUARD_CPU: KernelCell<Option<&'static dyn Cpu>> = KernelCell::new(None);

pub(crate) fn init_cpu(cpu: &'static dyn Cpu) {
    GUARD_CPU.replace(Some(cpu));
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicBool, AtomicUsize};

    struct StubCpu {
        enabled: AtomicBool,
        enables: AtomicUsize,
        disables: AtomicUsize,
    }

    impl Cpu for StubCpu {
        fn setup(&self) {}
        fn enable_interrupts(&self) {
            self.enables.fetch_add(1, Relaxed);
            self.enabled.store(true, Relaxed);
        }
        fn disable_interrupts(&self) {
            self.disables.fetch_add(1, Relaxed);
            self.enabled.store(false, Relaxed);
        }
        fn are_interrupts_enabled(&self) -> bool {
            self.enabled.load(Relaxed)
        }
        fn initialize_stack(&self, sp: usize, _entry: usize, _p1: usize, _p2: usize) -> usize {
            sp
        }
        fn swap_context(&self, _state: *mut usize, _store: *mut usize, _load: usize) {}
        fn get_system_time(&self) -> u64 {
            0
        }
        fn halt(&self) {}
    }

    fn leak_stub(start_enabled: bool) -> &'static StubCpu {
        Box::leak(Box::new(StubCpu {
            enabled: AtomicBool::new(start_enabled),
            enables: AtomicUsize::new(0),
            disables: AtomicUsize::new(0),
        }))
    }

    fn leak_cell() -> &'static IrqCell {
        Box::leak(Box::new(IrqCell::new("test-cell")))
    }

    fn locked_cpu() -> std::sync::MutexGuard<'static, ()> {
        static SLOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SLOT_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn acquire_masks_and_drop_restores_enabled_state() {
        let _lock = locked_cpu();
        let cpu = leak_stub(true);
        init_cpu(cpu);
        let cell = leak_cell();
        {
            let guard = cell.acquire();
            assert!(!cpu.are_interrupts_enabled());
            assert_eq!(cpu.disables.load(Relaxed), 1);
            assert!(guard.saved_if);
        }
        assert!(cpu.are_interrupts_enabled());
        assert_eq!(cpu.enables.load(Relaxed), 1);
    }

    #[test]
    fn acquire_from_masked_context_does_not_re_enable() {
        let _lock = locked_cpu();
        let cpu = leak_stub(false);
        init_cpu(cpu);
        let cell = leak_cell();
        {
            let guard = cell.acquire();
            assert!(!guard.saved_if);
        }
        assert_eq!(cpu.enables.load(Relaxed), 0);
    }

    #[test]
    fn reentrant_acquire_trips_and_releases_on_outer_drop() {
        let _lock = locked_cpu();
        let cpu = leak_stub(false);
        init_cpu(cpu);
        let cell = leak_cell();
        let outer = cell.acquire();
        assert_eq!(cell.depth.load(Relaxed), 1);
        {
            let _inner = cell.acquire();
            assert_eq!(cell.trip_budget.load(Relaxed), 2);
            assert_eq!(cell.depth.load(Relaxed), 2);
        }
        assert_eq!(cell.depth.load(Relaxed), 1);
        drop(outer);
        assert_eq!(cell.depth.load(Relaxed), 0);
    }

    #[test]
    fn trip_budget_saturates_and_stops_printing() {
        let _lock = locked_cpu();
        let cpu = leak_stub(false);
        init_cpu(cpu);
        let cell = leak_cell();
        let outer = cell.acquire();
        for expected in [2, 1, 0, 0] {
            let inner = cell.acquire();
            assert_eq!(cell.trip_budget.load(Relaxed), expected);
            drop(inner);
        }
        drop(outer);
    }

    #[test]
    fn acquire_tracks_held_with_masked_stub() {
        let _lock = locked_cpu();
        let cpu = leak_stub(false);
        init_cpu(cpu);
        let cell = leak_cell();
        {
            let guard = cell.acquire();
            assert!(!guard.saved_if);
            assert_eq!(cell.depth.load(Relaxed), 1);
        }
        assert_eq!(cell.depth.load(Relaxed), 0);
    }
}
