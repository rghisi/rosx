use crate::handle::TaskHandle;
use crate::kernel_cell::KernelCell;
use system::error::ErrorCode;

pub trait PortIo {
    fn read(&self, port: u16, width: PortWidth) -> u32;
    fn write(&self, port: u16, value: u32, width: PortWidth);
    fn read_words(&self, port: u16, destination: &mut [u16]);
    fn write_words(&self, port: u16, source: &[u16]);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(usize)]
pub enum PortWidth {
    Byte = 1,
    Word = 2,
    DWord = 4,
}

impl PortWidth {
    pub fn from_bytes(raw: usize) -> Option<PortWidth> {
        match raw {
            1 => Some(PortWidth::Byte),
            2 => Some(PortWidth::Word),
            4 => Some(PortWidth::DWord),
            _ => None,
        }
    }

    pub const fn bytes(self) -> usize {
        self as usize
    }
}

const GRANT_CAPACITY: usize = 8;
pub(crate) const BLOCK_WORDS_MAX: usize = 32768;
const PORT_SPACE: usize = 0x10000;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PortRange {
    pub base: u16,
    pub end: u16,
}

impl PortRange {
    pub const fn new(base: u16, end: u16) -> PortRange {
        PortRange { base, end }
    }

    fn contains(self, first: u16, last: u16) -> bool {
        u32::from(self.base) <= u32::from(first) && u32::from(last) <= u32::from(self.end)
    }
}

#[derive(Copy, Clone)]
struct Grant {
    task: TaskHandle,
    base: u16,
    end: u16,
}

impl Grant {
    fn covers(self, first: u16, last: u16) -> bool {
        u32::from(self.base) <= u32::from(first) && u32::from(last) <= u32::from(self.end)
    }

    fn overlaps(self, first: u16, last: u16) -> bool {
        u32::from(self.base) <= u32::from(last) && u32::from(first) <= u32::from(self.end)
    }
}

pub(crate) struct GrantTable {
    entries: [Option<Grant>; GRANT_CAPACITY],
}

impl GrantTable {
    pub(crate) const fn new() -> GrantTable {
        GrantTable {
            entries: [None; GRANT_CAPACITY],
        }
    }

    pub(crate) fn acquire(
        &mut self,
        task: TaskHandle,
        base: u16,
        end: u16,
        pool: &[PortRange],
    ) -> Result<(), ErrorCode> {
        if !pool.iter().any(|entry| entry.contains(base, end)) {
            return Err(ErrorCode::PortRangeNotInPool);
        }
        if self
            .entries
            .iter()
            .flatten()
            .any(|held| held.task == task && held.covers(base, end))
        {
            return Ok(());
        }
        if self
            .entries
            .iter()
            .flatten()
            .any(|held| held.task != task && held.overlaps(base, end))
        {
            return Err(ErrorCode::PortRangeBusy);
        }
        for slot in self.entries.iter_mut() {
            if slot.is_none() {
                *slot = Some(Grant { task, base, end });
                return Ok(());
            }
        }
        Err(ErrorCode::PortGrantTableFull)
    }

    pub(crate) fn release(&mut self, task: TaskHandle, base: u16, end: u16) -> Result<(), ErrorCode> {
        for slot in self.entries.iter_mut() {
            if matches!(slot, Some(held) if held.task == task && held.base == base && held.end == end) {
                *slot = None;
                return Ok(());
            }
        }
        Err(ErrorCode::PortAccessDenied)
    }

    pub(crate) fn revoke(&mut self, task: TaskHandle) {
        for slot in self.entries.iter_mut() {
            if matches!(slot, Some(existing) if existing.task == task) {
                *slot = None;
            }
        }
    }

    pub(crate) fn covers(&self, task: TaskHandle, first: u16, last: u16) -> bool {
        self.entries.iter().flatten().any(|existing| {
            existing.task == task
                && u32::from(existing.base) <= u32::from(first)
                && u32::from(last) <= u32::from(existing.end)
        })
    }
}

static GRANTS: KernelCell<GrantTable> = KernelCell::new(GrantTable::new());

pub(crate) fn revoke(task: TaskHandle) {
    GRANTS.borrow_mut().revoke(task);
}

pub(crate) fn authorize_acquire(
    caller: TaskHandle,
    raw_base: usize,
    raw_end: usize,
    pool: &[PortRange],
) -> Result<(), ErrorCode> {
    let (base, end) = validate_pair(raw_base, raw_end)?;
    GRANTS.borrow_mut().acquire(caller, base, end, pool)
}

pub(crate) fn authorize_release(
    caller: TaskHandle,
    raw_base: usize,
    raw_end: usize,
) -> Result<(), ErrorCode> {
    let (base, end) = validate_pair(raw_base, raw_end)?;
    GRANTS.borrow_mut().release(caller, base, end)
}

fn validate_pair(raw_base: usize, raw_end: usize) -> Result<(u16, u16), ErrorCode> {
    if raw_base >= PORT_SPACE || raw_end >= PORT_SPACE || raw_base > raw_end {
        return Err(ErrorCode::PortRequestInvalid);
    }
    Ok((raw_base as u16, raw_end as u16))
}

pub(crate) fn validate_single(raw_port: usize, raw_width: usize) -> Result<(u16, PortWidth), ErrorCode> {
    let Some(width) = PortWidth::from_bytes(raw_width) else {
        return Err(ErrorCode::PortRequestInvalid);
    };
    validate_span(raw_port, width.bytes())?;
    Ok((raw_port as u16, width))
}

pub(crate) fn validate_block(raw_port: usize, raw_count: usize) -> Result<(u16, usize), ErrorCode> {
    validate_span(raw_port, PortWidth::Word.bytes())?;
    if raw_count == 0 || raw_count > BLOCK_WORDS_MAX {
        return Err(ErrorCode::PortRequestInvalid);
    }
    Ok((raw_port as u16, raw_count))
}

fn validate_span(raw_port: usize, width_bytes: usize) -> Result<(), ErrorCode> {
    if raw_port >= PORT_SPACE || raw_port + width_bytes > PORT_SPACE {
        return Err(ErrorCode::PortRequestInvalid);
    }
    Ok(())
}

pub(crate) fn authorize_single(task: TaskHandle, raw_port: usize, raw_width: usize) -> Result<(u16, PortWidth), ErrorCode> {
    let (port, width) = validate_single(raw_port, raw_width)?;
    let last = port as usize + width.bytes() - 1;
    if GRANTS.borrow().covers(task, port, last as u16) {
        Ok((port, width))
    } else {
        Err(ErrorCode::PortAccessDenied)
    }
}

pub(crate) fn authorize_block(task: TaskHandle, raw_port: usize, raw_count: usize) -> Result<(u16, usize), ErrorCode> {
    let (port, count) = validate_block(raw_port, raw_count)?;
    if GRANTS.borrow().covers(task, port, port + 1) {
        Ok((port, count))
    } else {
        Err(ErrorCode::PortAccessDenied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY_DATA_BASE: u16 = 0x1F0;
    const PRIMARY_DATA_END: u16 = 0x1F7;
    const ALTERNATE_STATUS: u16 = 0x3F6;
    const UNGRANTED_PORT: u16 = 0x2F8;
    const WIDE_WIDTH: usize = 3;
    const ONE_WORD: usize = 1;

    const ATA_POOL: &[PortRange] = &[
        PortRange::new(PRIMARY_DATA_BASE, PRIMARY_DATA_END),
        PortRange::new(ALTERNATE_STATUS, ALTERNATE_STATUS),
    ];
    const PARTIAL_POOL: &[PortRange] = &[PortRange::new(PRIMARY_DATA_BASE, 0x1F6)];
    const WIDE_POOL: &[PortRange] = &[
        PortRange::new(0x300, 0x300),
        PortRange::new(0x301, 0x301),
        PortRange::new(0x302, 0x302),
        PortRange::new(0x303, 0x303),
        PortRange::new(0x304, 0x304),
        PortRange::new(0x305, 0x305),
        PortRange::new(0x306, 0x306),
        PortRange::new(0x307, 0x307),
        PortRange::new(0x308, 0x308),
    ];

    fn task(index: u16, generation: u16) -> TaskHandle {
        TaskHandle::new(index, generation)
    }

    fn reset_grants() {
        GRANTS.replace(GrantTable::new());
    }

    fn acquire_both(task_handle: TaskHandle) {
        let base = PRIMARY_DATA_BASE as usize;
        let end = PRIMARY_DATA_END as usize;
        let status = ALTERNATE_STATUS as usize;
        assert!(authorize_acquire(task_handle, base, end, ATA_POOL).is_ok());
        assert!(authorize_acquire(task_handle, status, status, ATA_POOL).is_ok());
    }

    #[test]
    fn acquire_admits_pool_span_and_authorizes_both_ends() {
        reset_grants();
        let handle = task(1, 0);
        acquire_both(handle);
        assert!(authorize_single(handle, 0x1F0, 1).is_ok());
        assert!(authorize_single(handle, 0x1F7, 1).is_ok());
        assert!(authorize_single(handle, 0x3F6, 1).is_ok());
    }

    #[test]
    fn acquire_subrange_of_a_pool_entry_is_admitted() {
        reset_grants();
        let handle = task(2, 0);
        assert!(authorize_acquire(handle, 0x1F0, 0x1F2, ATA_POOL).is_ok());
        assert!(authorize_single(handle, 0x1F2, 1).is_ok());
        assert_eq!(
            authorize_single(handle, 0x1F3, 1).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
    }

    #[test]
    fn acquire_of_a_range_outside_the_pool_is_rejected() {
        reset_grants();
        let handle = task(3, 0);
        assert_eq!(
            authorize_acquire(handle, UNGRANTED_PORT as usize, UNGRANTED_PORT as usize, ATA_POOL),
            Err(ErrorCode::PortRangeNotInPool)
        );
    }

    #[test]
    fn acquire_straddling_two_pool_entries_is_rejected() {
        reset_grants();
        let handle = task(4, 0);
        assert_eq!(
            authorize_acquire(handle, PRIMARY_DATA_BASE as usize, ALTERNATE_STATUS as usize, ATA_POOL),
            Err(ErrorCode::PortRangeNotInPool)
        );
    }

    #[test]
    fn acquire_overlapping_another_tasks_grant_is_busy() {
        reset_grants();
        let holder = task(5, 0);
        let rival = task(6, 0);
        acquire_both(holder);
        assert_eq!(
            authorize_acquire(rival, PRIMARY_DATA_BASE as usize, PRIMARY_DATA_END as usize, ATA_POOL),
            Err(ErrorCode::PortRangeBusy)
        );
        assert_eq!(
            authorize_acquire(rival, 0x1F4, 0x1F4, ATA_POOL),
            Err(ErrorCode::PortRangeBusy)
        );
    }

    #[test]
    fn acquire_by_the_holder_already_covering_the_range_is_idempotent() {
        reset_grants();
        let handle = task(7, 0);
        acquire_both(handle);
        assert!(authorize_acquire(handle, 0x1F2, 0x1F4, ATA_POOL).is_ok());
        assert!(authorize_acquire(handle, PRIMARY_DATA_BASE as usize, PRIMARY_DATA_END as usize, ATA_POOL).is_ok());
        assert_eq!(
            authorize_acquire(task(8, 0), PRIMARY_DATA_BASE as usize, PRIMARY_DATA_END as usize, ATA_POOL),
            Err(ErrorCode::PortRangeBusy)
        );
    }

    #[test]
    fn release_hands_the_range_to_the_next_holder() {
        reset_grants();
        let holder = task(9, 0);
        let successor = task(10, 0);
        acquire_both(holder);
        let status = ALTERNATE_STATUS as usize;
        assert!(authorize_release(holder, status, status).is_ok());
        assert!(authorize_acquire(successor, status, status, ATA_POOL).is_ok());
        assert_eq!(
            authorize_single(holder, ALTERNATE_STATUS as usize, 1).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
        assert_eq!(
            authorize_acquire(successor, PRIMARY_DATA_BASE as usize, PRIMARY_DATA_END as usize, ATA_POOL),
            Err(ErrorCode::PortRangeBusy)
        );
    }

    #[test]
    fn release_of_a_partial_or_foreign_span_is_denied() {
        reset_grants();
        let holder = task(11, 0);
        let stranger = task(12, 0);
        acquire_both(holder);
        assert_eq!(
            authorize_release(holder, PRIMARY_DATA_BASE as usize, 0x1F2),
            Err(ErrorCode::PortAccessDenied)
        );
        assert_eq!(
            authorize_release(stranger, PRIMARY_DATA_BASE as usize, PRIMARY_DATA_END as usize),
            Err(ErrorCode::PortAccessDenied)
        );
        assert!(authorize_single(holder, 0x1F0, 1).is_ok());
    }

    #[test]
    fn acquire_pair_malformed_is_invalid() {
        reset_grants();
        let handle = task(13, 0);
        assert_eq!(
            authorize_acquire(handle, 0x1F7, PRIMARY_DATA_BASE as usize, ATA_POOL),
            Err(ErrorCode::PortRequestInvalid)
        );
        assert_eq!(
            authorize_acquire(handle, PORT_SPACE, PORT_SPACE, ATA_POOL),
            Err(ErrorCode::PortRequestInvalid)
        );
    }

    #[test]
    fn single_access_straddling_grant_edge_is_denied() {
        reset_grants();
        let handle = task(2, 0);
        assert!(authorize_acquire(handle, PRIMARY_DATA_BASE as usize, 0x1F6, PARTIAL_POOL).is_ok());
        assert_eq!(
            authorize_single(handle, 0x1F6, 4).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
    }

    #[test]
    fn four_byte_access_fits_exactly_at_grant_end() {
        reset_grants();
        let handle = task(3, 0);
        acquire_both(handle);
        assert!(authorize_single(handle, 0x1F4, 4).is_ok());
        assert_eq!(
            authorize_single(handle, 0x1F5, 4).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
    }

    #[test]
    fn block_access_requires_grant_covering_the_word() {
        reset_grants();
        let handle = task(4, 0);
        assert!(authorize_acquire(handle, PRIMARY_DATA_BASE as usize, 0x1F2, PARTIAL_POOL).is_ok());
        assert!(authorize_block(handle, 0x1F0, ONE_WORD).is_ok());
        assert!(authorize_block(handle, 0x1F1, ONE_WORD).is_ok());
        assert_eq!(
            authorize_block(handle, 0x1F2, ONE_WORD).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
        assert!(authorize_block(handle, 0x1F0, BLOCK_WORDS_MAX).is_ok());
    }

    #[test]
    fn block_at_grant_end_reaches_beyond_the_grant() {
        reset_grants();
        let handle = task(5, 0);
        acquire_both(handle);
        assert_eq!(
            authorize_block(handle, 0x1F7, ONE_WORD).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
    }

    #[test]
    fn ungranted_task_is_denied_every_access() {
        reset_grants();
        let granted = task(6, 0);
        let outsider = task(7, 0);
        acquire_both(granted);
        assert_eq!(
            authorize_single(outsider, 0x1F0, 1).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
        assert_eq!(
            authorize_block(outsider, 0x1F0, ONE_WORD).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
        assert_eq!(
            authorize_single(outsider, UNGRANTED_PORT as usize, 1).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
    }

    #[test]
    fn recycled_slot_with_new_generation_misses_the_grant() {
        reset_grants();
        let dead = task(8, 3);
        assert!(authorize_acquire(dead, PRIMARY_DATA_BASE as usize, PRIMARY_DATA_END as usize, ATA_POOL).is_ok());
        let reborn = task(8, 4);
        assert_eq!(
            authorize_single(reborn, 0x1F0, 1).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
        assert!(authorize_single(dead, 0x1F0, 1).is_ok());
    }

    #[test]
    fn revoke_removes_every_entry_of_the_task() {
        reset_grants();
        let handle = task(9, 0);
        acquire_both(handle);
        revoke(handle);
        assert_eq!(
            authorize_single(handle, 0x1F0, 1).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
        assert_eq!(
            authorize_single(handle, 0x3F6, 1).unwrap_err(),
            ErrorCode::PortAccessDenied
        );
    }

    #[test]
    fn revoke_frees_the_range_for_the_next_task() {
        reset_grants();
        let holder = task(14, 0);
        let successor = task(15, 0);
        acquire_both(holder);
        revoke(holder);
        assert!(authorize_acquire(successor, PRIMARY_DATA_BASE as usize, PRIMARY_DATA_END as usize, ATA_POOL).is_ok());
    }

    #[test]
    fn acquire_beyond_capacity_fails() {
        reset_grants();
        for index in 0..GRANT_CAPACITY {
            let port = 0x300 + index;
            assert!(authorize_acquire(task(20 + index as u16, 0), port, port, WIDE_POOL).is_ok());
        }
        assert_eq!(
            authorize_acquire(task(99, 0), 0x308, 0x308, WIDE_POOL),
            Err(ErrorCode::PortGrantTableFull)
        );
    }

    #[test]
    fn width_outside_one_two_four_is_invalid() {
        for raw_width in [0usize, WIDE_WIDTH, 8] {
            assert_eq!(
                validate_single(0x1F0, raw_width).unwrap_err(),
                ErrorCode::PortRequestInvalid
            );
        }
        assert_eq!(PortWidth::from_bytes(1).unwrap().bytes(), 1);
        assert_eq!(PortWidth::from_bytes(2).unwrap().bytes(), 2);
        assert_eq!(PortWidth::from_bytes(4).unwrap().bytes(), 4);
    }

    #[test]
    fn block_count_bounds_reject_zero_and_overflow() {
        assert_eq!(
            validate_block(0x1F0, 0).unwrap_err(),
            ErrorCode::PortRequestInvalid
        );
        assert_eq!(
            validate_block(0x1F0, BLOCK_WORDS_MAX + 1).unwrap_err(),
            ErrorCode::PortRequestInvalid
        );
        assert!(validate_block(0x1F0, BLOCK_WORDS_MAX).is_ok());
    }

    #[test]
    fn port_plus_width_wrap_or_beyond_is_invalid() {
        assert_eq!(
            validate_single(0x1_0000, 1).unwrap_err(),
            ErrorCode::PortRequestInvalid
        );
        assert_eq!(
            validate_single(0xFFFF, 2).unwrap_err(),
            ErrorCode::PortRequestInvalid
        );
        assert_eq!(
            validate_single(0xFFFE, 4).unwrap_err(),
            ErrorCode::PortRequestInvalid
        );
        assert_eq!(
            validate_block(0xFFFF, ONE_WORD).unwrap_err(),
            ErrorCode::PortRequestInvalid
        );
        assert!(validate_single(0xFFFC, 4).is_ok());
        assert!(validate_block(0xFFFE, ONE_WORD).is_ok());
    }

    #[test]
    fn malformed_request_is_rejected_before_grant_consultation() {
        reset_grants();
        let handle = task(10, 0);
        acquire_both(handle);
        assert_eq!(
            authorize_single(handle, 0x1F0, WIDE_WIDTH).unwrap_err(),
            ErrorCode::PortRequestInvalid
        );
        assert_eq!(
            authorize_block(handle, 0x1F0, 0).unwrap_err(),
            ErrorCode::PortRequestInvalid
        );
    }
}
