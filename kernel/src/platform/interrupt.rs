use crate::kernel_cell::KernelCell;
use alloc::boxed::Box;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    Edge,
    Level,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    High,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptError {
    InvalidIrq,
    Unsupported,
}

pub type Result = core::result::Result<(), InterruptError>;

pub trait InterruptController {
    const MAX_IRQ: u32;
    const VECTOR_BASE: u8;

    fn new() -> Self;
    fn configure(
        &mut self,
        irq: u32,
        vector: u8,
        trigger: TriggerMode,
        polarity: Polarity,
    ) -> Result;
    fn mask(&mut self, irq: u32) -> Result;
    fn unmask(&mut self, irq: u32) -> Result;
    fn eoi(&mut self, irq: u32);
}

type HwHandler = fn(irq: u32);

#[derive(Clone, Copy)]
struct HandlerEntry {
    handler: HwHandler,
    auto_unmask: bool,
}

const HANDLED_IRQS: usize = 16;

fn ignore_irq(_irq: u32) {}

fn reject_irq(_irq: u32) -> Result {
    Err(InterruptError::InvalidIrq)
}

fn reject_configure(_irq: u32, _trigger: TriggerMode, _polarity: Polarity) -> Result {
    Err(InterruptError::InvalidIrq)
}

static CONTROLLER_ADDRESS: KernelCell<Option<usize>> = KernelCell::new(None);
static DISPATCH: KernelCell<fn(irq: u32)> = KernelCell::new(ignore_irq);
static UNMASK: KernelCell<fn(irq: u32) -> Result> = KernelCell::new(reject_irq);
static CONFIGURE: KernelCell<fn(irq: u32, trigger: TriggerMode, polarity: Polarity) -> Result> =
    KernelCell::new(reject_configure);
static HANDLERS: KernelCell<[Option<HandlerEntry>; HANDLED_IRQS]> =
    KernelCell::new([None; HANDLED_IRQS]);

/// Binds the kernel dispatch core to the concrete controller type `C`.
///
/// # Safety
/// The leaked instance address is later cast back to `&'static KernelCell<C>`.
/// The cast is sound because the address is stored here only by this function
/// and read back exclusively through the `dispatch_via::<C>` / `unmask_via::<C>`
/// pointers installed together with it, so the pointer and its type parameter
/// always agree. Must be called once, before interrupts are enabled.
pub fn install<C: InterruptController + 'static>() {
    let controller: &'static KernelCell<C> = Box::leak(Box::new(KernelCell::new(C::new())));
    CONTROLLER_ADDRESS.replace(Some(controller as *const KernelCell<C> as usize));
    DISPATCH.replace(dispatch_via::<C>);
    UNMASK.replace(unmask_via::<C>);
    CONFIGURE.replace(configure_via::<C>);
}

pub fn unmask(irq: u32) -> Result {
    (UNMASK.borrow())(irq)
}

pub fn configure(irq: u32, trigger: TriggerMode, polarity: Polarity) -> Result {
    (CONFIGURE.borrow())(irq, trigger, polarity)
}

pub fn subscribe(irq: u32) -> Result {
    if !system::irq::is_subscribable(irq) {
        return Err(InterruptError::InvalidIrq);
    }
    configure(irq, TriggerMode::Level, Polarity::High)?;
    register_entry(irq, deliver_irq_event, false)?;
    unmask(irq)
}

pub fn clear(irq: u32) -> Result {
    if !system::irq::is_subscribable(irq) {
        return Err(InterruptError::InvalidIrq);
    }
    if !HANDLERS.borrow()[irq as usize].is_some() {
        return Err(InterruptError::InvalidIrq);
    }
    unmask(irq)
}

fn deliver_irq_event(irq: u32) {
    let frame = system::irq::encode_irq_event(irq);
    let message = system::ipc::Message::new(system::ipc::KERNEL_EVENT_CONN, &frame);
    crate::ipc::deliver_to_service(system::irq::SOURCE_NAMES[irq as usize], message);
}

pub fn register_handler(irq: u32, handler: HwHandler) -> Result {
    register_entry(irq, handler, true)
}

fn register_entry(irq: u32, handler: HwHandler, auto_unmask: bool) -> Result {
    if irq as usize >= HANDLED_IRQS {
        return Err(InterruptError::InvalidIrq);
    }
    HANDLERS.borrow_mut()[irq as usize] = Some(HandlerEntry {
        handler,
        auto_unmask,
    });
    Ok(())
}

pub fn dispatch_irq(irq: u32) {
    (DISPATCH.borrow())(irq)
}

fn controller<C: InterruptController + 'static>() -> Option<&'static KernelCell<C>> {
    match CONTROLLER_ADDRESS.borrow() {
        Some(address) => Some(unsafe { &*(*address as *const KernelCell<C>) }),
        None => None,
    }
}

fn dispatch_via<C: InterruptController + 'static>(irq: u32) {
    let Some(controller) = controller::<C>() else {
        return;
    };
    let _ = C::mask(controller.borrow_mut(), irq);
    C::eoi(controller.borrow_mut(), irq);
    let entry = HANDLERS.borrow().get(irq as usize).copied().flatten();
    match entry {
        Some(entry) => {
            (entry.handler)(irq);
            if entry.auto_unmask {
                let _ = C::unmask(controller.borrow_mut(), irq);
            }
        }
        None => {}
    }
}

fn unmask_via<C: InterruptController + 'static>(irq: u32) -> Result {
    match controller::<C>() {
        Some(controller) => C::unmask(controller.borrow_mut(), irq),
        None => Err(InterruptError::InvalidIrq),
    }
}

fn configure_via<C: InterruptController + 'static>(
    irq: u32,
    trigger: TriggerMode,
    polarity: Polarity,
) -> Result {
    let Some(controller) = controller::<C>() else {
        return Err(InterruptError::InvalidIrq);
    };
    let Some(vector) = C::VECTOR_BASE.checked_add(irq as u8) else {
        return Err(InterruptError::InvalidIrq);
    };
    C::configure(controller.borrow_mut(), irq, vector, trigger, polarity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const TIMER_IRQ: u32 = 0;
    const SECOND_IRQ: u32 = 1;
    const UNREGISTERED_IRQ: u32 = 5;
    const OUT_OF_RANGE_IRQ: u32 = 99;

    static OP_LOG: KernelCell<[u8; 64]> = KernelCell::new([0; 64]);
    static OP_COUNT: AtomicUsize = AtomicUsize::new(0);
    static MASK_FAILURE: AtomicBool = AtomicBool::new(false);

    const NEW_OP: u8 = 1;
    const MASK_OP: u8 = 2;
    const UNMASK_OP: u8 = 3;
    const EOI_OP: u8 = 4;
    const HANDLER_OP: u8 = 5;
    const CONFIGURE_OP: u8 = 6;

    static CONFIGURED_VECTOR: KernelCell<u8> = KernelCell::new(0);

    struct MockController;

    impl InterruptController for MockController {
        const MAX_IRQ: u32 = 15;
        const VECTOR_BASE: u8 = 0x20;

        fn new() -> Self {
            record(NEW_OP);
            MockController
        }

        fn configure(
            &mut self,
            irq: u32,
            vector: u8,
            _trigger: TriggerMode,
            _polarity: Polarity,
        ) -> Result {
            check_irq(irq)?;
            CONFIGURED_VECTOR.replace(vector);
            record(CONFIGURE_OP);
            Ok(())
        }

        fn mask(&mut self, irq: u32) -> Result {
            check_irq(irq)?;
            if MASK_FAILURE.load(Ordering::Relaxed) {
                return Err(InterruptError::Unsupported);
            }
            record(MASK_OP);
            Ok(())
        }

        fn unmask(&mut self, irq: u32) -> Result {
            check_irq(irq)?;
            record(UNMASK_OP);
            Ok(())
        }

        fn eoi(&mut self, irq: u32) {
            if irq <= Self::MAX_IRQ {
                record(EOI_OP);
            }
        }
    }

    fn check_irq(irq: u32) -> Result {
        if irq <= MockController::MAX_IRQ {
            Ok(())
        } else {
            Err(InterruptError::InvalidIrq)
        }
    }

    fn record(op: u8) {
        let index = OP_COUNT.fetch_add(1, Ordering::SeqCst);
        if index < OP_LOG.borrow().len() {
            OP_LOG.borrow_mut()[index] = op;
        }
    }

    fn recorded_ops() -> alloc::vec::Vec<u8> {
        let count = OP_COUNT.load(Ordering::SeqCst).min(OP_LOG.borrow().len());
        OP_LOG.borrow()[..count].to_vec()
    }

    fn mock_handler(_irq: u32) {
        record(HANDLER_OP);
    }

    fn reset_state() {
        OP_COUNT.store(0, Ordering::SeqCst);
        MASK_FAILURE.store(false, Ordering::Relaxed);
        HANDLERS.replace([None; HANDLED_IRQS]);
        CONTROLLER_ADDRESS.replace(None);
        DISPATCH.replace(ignore_irq);
        UNMASK.replace(reject_irq);
        CONFIGURE.replace(reject_configure);
        CONFIGURED_VECTOR.replace(0);
    }

    #[test]
    fn dispatch_masks_then_eoi_then_handles_then_rearms() {
        reset_state();
        install::<MockController>();
        register_handler(TIMER_IRQ, mock_handler).unwrap();
        assert_eq!(recorded_ops(), alloc::vec![NEW_OP]);

        dispatch_irq(TIMER_IRQ);

        assert_eq!(
            recorded_ops(),
            alloc::vec![NEW_OP, MASK_OP, EOI_OP, HANDLER_OP, UNMASK_OP]
        );
    }

    #[test]
    fn dispatch_of_unregistered_irq_masks_and_eo_is_without_handler() {
        reset_state();
        install::<MockController>();

        dispatch_irq(UNREGISTERED_IRQ);

        assert_eq!(recorded_ops(), alloc::vec![NEW_OP, MASK_OP, EOI_OP]);
    }

    #[test]
    fn dispatch_of_out_of_range_irq_reaches_nothing() {
        reset_state();
        install::<MockController>();

        dispatch_irq(OUT_OF_RANGE_IRQ);

        assert_eq!(recorded_ops(), alloc::vec![NEW_OP]);
    }

    #[test]
    fn dispatch_without_installed_controller_is_ignored() {
        reset_state();

        dispatch_irq(TIMER_IRQ);

        assert_eq!(recorded_ops(), alloc::vec![]);
    }

    #[test]
    fn failed_mask_still_signals_eoi_before_handler() {
        reset_state();
        install::<MockController>();
        register_handler(TIMER_IRQ, mock_handler).unwrap();
        MASK_FAILURE.store(true, Ordering::Relaxed);

        dispatch_irq(TIMER_IRQ);

        assert_eq!(
            recorded_ops(),
            alloc::vec![NEW_OP, EOI_OP, HANDLER_OP, UNMASK_OP]
        );
    }

    #[test]
    fn register_handler_rejects_irq_beyond_handled_range() {
        reset_state();

        assert_eq!(
            register_handler(HANDLED_IRQS as u32, mock_handler),
            Err(InterruptError::InvalidIrq)
        );
    }

    #[test]
    fn unmask_before_install_is_invalid() {
        reset_state();

        assert_eq!(unmask(TIMER_IRQ), Err(InterruptError::InvalidIrq));
    }

    #[test]
    fn unmask_after_install_reaches_controller() {
        reset_state();
        install::<MockController>();

        assert_eq!(unmask(SECOND_IRQ), Ok(()));
        assert_eq!(recorded_ops(), alloc::vec![NEW_OP, UNMASK_OP]);
    }

    const SUBSCRIBABLE_IRQ: u32 = 9;

    #[test]
    fn subscribe_configures_then_registers_then_unmasks() {
        reset_state();
        install::<MockController>();

        assert_eq!(subscribe(SUBSCRIBABLE_IRQ), Ok(()));

        assert_eq!(
            recorded_ops(),
            alloc::vec![NEW_OP, CONFIGURE_OP, UNMASK_OP]
        );
        assert_eq!(
            *CONFIGURED_VECTOR.borrow(),
            MockController::VECTOR_BASE + SUBSCRIBABLE_IRQ as u8
        );
        assert!(HANDLERS.borrow()[SUBSCRIBABLE_IRQ as usize].is_some());
    }

    #[test]
    fn dispatch_of_subscribed_irq_stays_masked_for_the_driver() {
        reset_state();
        install::<MockController>();
        subscribe(SUBSCRIBABLE_IRQ).unwrap();

        dispatch_irq(SUBSCRIBABLE_IRQ);

        assert_eq!(
            recorded_ops(),
            alloc::vec![NEW_OP, CONFIGURE_OP, UNMASK_OP, MASK_OP, EOI_OP]
        );
    }

    #[test]
    fn clear_rearms_a_subscribed_line_and_refuses_the_unsubscribed() {
        reset_state();
        install::<MockController>();
        subscribe(SUBSCRIBABLE_IRQ).unwrap();

        assert_eq!(clear(SUBSCRIBABLE_IRQ), Ok(()));
        assert_eq!(clear(SECOND_IRQ), Err(InterruptError::InvalidIrq));
        assert_eq!(
            recorded_ops(),
            alloc::vec![NEW_OP, CONFIGURE_OP, UNMASK_OP, UNMASK_OP]
        );
    }

    #[test]
    fn subscribe_refuses_reserved_and_out_of_range_lines() {
        reset_state();
        install::<MockController>();

        for irq in [0, 1, 2, 8, 16, 99] {
            assert_eq!(subscribe(irq), Err(InterruptError::InvalidIrq));
            assert!(HANDLERS.borrow()[..].iter().all(|slot| slot.is_none()));
        }
        assert_eq!(recorded_ops(), alloc::vec![NEW_OP]);
    }

    #[test]
    fn subscribe_before_install_registers_nothing() {
        reset_state();

        assert_eq!(subscribe(SUBSCRIBABLE_IRQ), Err(InterruptError::InvalidIrq));
        assert!(HANDLERS.borrow()[SUBSCRIBABLE_IRQ as usize].is_none());
    }
}
