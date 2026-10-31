use system::error::{ERROR_BIT, is_token};

pub(crate) const INDEX_BITS: usize = 16;
pub(crate) const INDEX_MASK: usize = (1 << INDEX_BITS) - 1;
pub(crate) const GENERATION_MASK: usize = (ERROR_BIT - 1) >> INDEX_BITS;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Handle {
    pub(crate) index: u16,
    pub(crate) generation: u16,
}

impl Handle {
    pub(crate) const fn new(index: u16, generation: u16) -> Self {
        debug_assert!(
            generation <= (GENERATION_MASK as u16),
            "generation outside 15-bit handle space"
        );
        Self { index, generation }
    }

    pub(crate) fn pack(&self) -> usize {
        debug_assert!(
            self.generation <= (GENERATION_MASK as u16),
            "generation outside 15-bit handle space"
        );
        (((self.generation as usize) & GENERATION_MASK) << INDEX_BITS)
            | (self.index as usize & INDEX_MASK)
    }

    pub(crate) fn split(packed: usize) -> Self {
        Self {
            index: (packed & INDEX_MASK) as u16,
            generation: ((packed >> INDEX_BITS) & GENERATION_MASK) as u16,
        }
    }
}

pub(crate) const fn next_generation(current: u16) -> u16 {
    (current + 1) & (GENERATION_MASK as u16)
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct TaskHandle(Handle);

#[allow(dead_code)]
impl TaskHandle {
    pub(crate) const fn new(index: u16, generation: u16) -> Self {
        Self(Handle::new(index, generation))
    }

    pub(crate) fn pack(self) -> usize {
        self.0.pack()
    }

    pub(crate) fn from_packed(raw: usize) -> Option<Self> {
        if is_token(raw) {
            Some(Self(Handle::split(raw)))
        } else {
            None
        }
    }

    pub(crate) fn index(self) -> u16 {
        self.0.index
    }

    pub(crate) fn generation(self) -> u16 {
        self.0.generation
    }
}

impl From<Handle> for TaskHandle {
    fn from(handle: Handle) -> Self {
        Self(handle)
    }
}

impl From<TaskHandle> for Handle {
    fn from(handle: TaskHandle) -> Self {
        handle.0
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct FutureHandle(Handle);

#[allow(dead_code)]
impl FutureHandle {
    pub(crate) const fn new(index: u16, generation: u16) -> Self {
        Self(Handle::new(index, generation))
    }

    pub(crate) fn pack(self) -> usize {
        self.0.pack()
    }

    pub(crate) fn from_packed(raw: usize) -> Option<Self> {
        if is_token(raw) {
            Some(Self(Handle::split(raw)))
        } else {
            None
        }
    }

    pub(crate) fn index(self) -> u16 {
        self.0.index
    }

    pub(crate) fn generation(self) -> u16 {
        self.0.generation
    }
}

impl From<Handle> for FutureHandle {
    fn from(handle: Handle) -> Self {
        Self(handle)
    }
}

impl From<FutureHandle> for Handle {
    fn from(handle: FutureHandle) -> Self {
        handle.0
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct IpcConnectionHandle(Handle);

#[allow(dead_code)]
impl IpcConnectionHandle {
    pub(crate) const fn new(index: u16, generation: u16) -> Self {
        Self(Handle::new(index, generation))
    }

    pub(crate) fn pack(self) -> usize {
        self.0.pack()
    }

    pub(crate) fn from_packed(raw: usize) -> Option<Self> {
        if is_token(raw) {
            Some(Self(Handle::split(raw)))
        } else {
            None
        }
    }

    pub(crate) fn index(self) -> u16 {
        self.0.index
    }

    pub(crate) fn generation(self) -> u16 {
        self.0.generation
    }
}

impl From<Handle> for IpcConnectionHandle {
    fn from(handle: Handle) -> Self {
        Self(handle)
    }
}

impl From<IpcConnectionHandle> for Handle {
    fn from(handle: IpcConnectionHandle) -> Self {
        handle.0
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct IpcBindingHandle(Handle);

#[allow(dead_code)]
impl IpcBindingHandle {
    pub(crate) const fn new(index: u16, generation: u16) -> Self {
        Self(Handle::new(index, generation))
    }

    pub(crate) fn pack(self) -> usize {
        self.0.pack()
    }

    pub(crate) fn from_packed(raw: usize) -> Option<Self> {
        if is_token(raw) {
            Some(Self(Handle::split(raw)))
        } else {
            None
        }
    }

    pub(crate) fn index(self) -> u16 {
        self.0.index
    }

    pub(crate) fn generation(self) -> u16 {
        self.0.generation
    }
}

impl From<Handle> for IpcBindingHandle {
    fn from(handle: Handle) -> Self {
        Self(handle)
    }
}

impl From<IpcBindingHandle> for Handle {
    fn from(handle: IpcBindingHandle) -> Self {
        handle.0
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct IpcDataHandle(Handle);

#[allow(dead_code)]
impl IpcDataHandle {
    pub(crate) const fn new(index: u16, generation: u16) -> Self {
        Self(Handle::new(index, generation))
    }

    pub(crate) fn pack(self) -> usize {
        self.0.pack()
    }

    pub(crate) fn from_packed(raw: usize) -> Option<Self> {
        if is_token(raw) {
            Some(Self(Handle::split(raw)))
        } else {
            None
        }
    }

    pub(crate) fn index(self) -> u16 {
        self.0.index
    }

    pub(crate) fn generation(self) -> u16 {
        self.0.generation
    }
}

impl From<Handle> for IpcDataHandle {
    fn from(handle: Handle) -> Self {
        Self(handle)
    }
}

impl From<IpcDataHandle> for Handle {
    fn from(handle: IpcDataHandle) -> Self {
        handle.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::arena::GenerationalArena;
    use system::error::ErrorCode;

    const ALL_ERROR_CODES: [ErrorCode; 21] = [
        ErrorCode::IpcServerNotFound,
        ErrorCode::IpcConnectionCannotBeEstablished,
        ErrorCode::IpcAlreadyBound,
        ErrorCode::IpcSendConnectionNotFound,
        ErrorCode::IpcSendConnectionCongested,
        ErrorCode::IpcReceiveConnectionNotFound,
        ErrorCode::IpcReceiveNoMessagesAvailable,
        ErrorCode::IpcMailboxNotAvailable,
        ErrorCode::IpcDataTooLarge,
        ErrorCode::IpcDataBudgetExceeded,
        ErrorCode::IpcDataNotFound,
        ErrorCode::IpcDataNotReadable,
        ErrorCode::WaitsetFull,
        ErrorCode::WaitsetChildInvalid,
        ErrorCode::WaitsetNestingForbidden,
        ErrorCode::WaitsetNotFound,
        ErrorCode::WaitsetLeafRequired,
        ErrorCode::FutureCapacityExhausted,
        ErrorCode::PortAccessDenied,
        ErrorCode::PortRequestInvalid,
        ErrorCode::Unknown,
    ];

    #[test]
    fn handle_pack_should_encode_generation_in_bits_30_to_16_and_index_in_bits_15_to_0() {
        assert_eq!(TaskHandle::new(5, 10).pack(), (10usize << 16) | 5);
    }

    #[test]
    fn handle_from_packed_should_decode_index_and_generation() {
        let packed = (10usize << 16) | 5;
        let handle = TaskHandle::from_packed(packed).unwrap();
        assert_eq!(handle.index(), 5);
        assert_eq!(handle.generation(), 10);
    }

    #[test]
    fn handle_pack_from_packed_should_roundtrip() {
        let original = TaskHandle::new(42, 7);
        assert_eq!(TaskHandle::from_packed(original.pack()), Some(original));
    }

    #[test]
    fn handle_pack_from_packed_should_roundtrip_edge_handles() {
        for (index, generation) in [(0u16, 0u16), (65535, 0), (0, 32767), (65535, 32767)] {
            let handle = TaskHandle::new(index, generation);
            assert_eq!(TaskHandle::from_packed(handle.pack()), Some(handle));
        }
    }

    #[test]
    fn handle_pack_should_never_set_error_bit() {
        let mut index = 0u16;
        loop {
            let mut generation = 0u16;
            loop {
                assert_eq!(TaskHandle::new(index, generation).pack() & ERROR_BIT, 0);
                if generation >= 0x7FFF {
                    break;
                }
                generation = generation.saturating_add(997).min(0x7FFF);
            }
            if index >= 65535 {
                break;
            }
            index = index.saturating_add(997).min(65535);
        }
    }

    #[test]
    fn from_packed_should_accept_only_wellformed_packed_values() {
        assert!(TaskHandle::from_packed(0).is_some());
        assert!(TaskHandle::from_packed(0x7FFF_FFFF).is_some());
        assert!(TaskHandle::from_packed(ERROR_BIT).is_none());
        assert!(TaskHandle::from_packed(ERROR_BIT | 1).is_none());
        assert!(TaskHandle::from_packed(1usize << 32).is_none());
        assert!(TaskHandle::from_packed(usize::MAX).is_none());
    }

    #[test]
    fn generation_counter_should_cycle_from_32767_to_0_without_setting_error_bit() {
        let mut arena: GenerationalArena<u8, 1> = GenerationalArena::new();
        for counter in 0..32768usize {
            let handle = TaskHandle::from(arena.add(0).unwrap());
            assert_eq!(handle.generation(), (counter & 0x7FFF) as u16);
            assert_eq!(handle.pack() & ERROR_BIT, 0);
            arena.remove(handle.into()).unwrap();
        }
        let wrapped = TaskHandle::from(arena.add(0).unwrap());
        assert_eq!(wrapped.generation(), 0);
        assert_eq!(wrapped.pack() & ERROR_BIT, 0);
    }

    #[test]
    fn next_generation_should_wrap_within_generation_space() {
        assert_eq!(next_generation(0), 1);
        assert_eq!(next_generation(0x7FFE), 0x7FFF);
        assert_eq!(next_generation(0x7FFF), 0);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "generation outside 15-bit handle space")]
    fn new_should_panic_on_generation_above_the_15_bit_domain() {
        TaskHandle::new(0, 0xFFFF);
    }

    #[test]
    fn handles_should_be_equals_when_created_with_the_same_index_and_generation() {
        let h1 = TaskHandle::new(5, 10);
        let h2 = TaskHandle::new(5, 10);
        let h3 = TaskHandle::new(5, 11);

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn edge_pair_codec_roundtrips_through_the_domain() {
        for (index, generation) in [(0u16, 0u16), (65535, 0), (0, 32767), (65535, 32767)] {
            let handle = TaskHandle::new(index, generation);
            let raw = handle.pack();
            assert!(is_token(raw));
            assert_eq!(TaskHandle::from_packed(raw), Some(handle));
        }
    }

    #[test]
    fn packed_handle_space_is_disjoint_from_error_space() {
        assert_eq!(TaskHandle::new(0xFFFF, 0x7FFF).pack(), 0x7FFF_FFFF);
        assert!(is_token(0x7FFF_FFFF));
        assert!(!is_token(ERROR_BIT));
        for code in ALL_ERROR_CODES {
            let raw = code.to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert!(!is_token(raw));
        }
    }

    #[test]
    fn ipc_data_handle_pack_should_encode_generation_in_bits_30_to_16_and_index_in_bits_15_to_0() {
        assert_eq!(IpcDataHandle::new(5, 10).pack(), (10usize << 16) | 5);
    }

    #[test]
    fn ipc_data_handle_pack_from_packed_should_roundtrip() {
        let original = IpcDataHandle::new(42, 7);
        assert_eq!(IpcDataHandle::from_packed(original.pack()), Some(original));
    }

    #[test]
    fn ipc_data_handle_pack_from_packed_should_roundtrip_edge_handles() {
        for (index, generation) in [(0u16, 0u16), (65535, 0), (0, 32767), (65535, 32767)] {
            let handle = IpcDataHandle::new(index, generation);
            assert_eq!(IpcDataHandle::from_packed(handle.pack()), Some(handle));
        }
    }

    #[test]
    fn ipc_data_handle_pack_should_never_set_error_bit() {
        let mut index = 0u16;
        loop {
            let mut generation = 0u16;
            loop {
                assert_eq!(IpcDataHandle::new(index, generation).pack() & ERROR_BIT, 0);
                if generation >= 0x7FFF {
                    break;
                }
                generation = generation.saturating_add(997).min(0x7FFF);
            }
            if index >= 65535 {
                break;
            }
            index = index.saturating_add(997).min(65535);
        }
    }

    #[test]
    fn ipc_data_handle_from_packed_should_accept_only_wellformed_packed_values() {
        assert!(IpcDataHandle::from_packed(0).is_some());
        assert!(IpcDataHandle::from_packed(0x7FFF_FFFF).is_some());
        assert!(IpcDataHandle::from_packed(ERROR_BIT).is_none());
        assert!(IpcDataHandle::from_packed(ERROR_BIT | 1).is_none());
        assert!(IpcDataHandle::from_packed(1usize << 32).is_none());
        assert!(IpcDataHandle::from_packed(usize::MAX).is_none());
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "generation outside 15-bit handle space")]
    fn ipc_data_handle_new_should_panic_on_generation_above_the_15_bit_domain() {
        IpcDataHandle::new(0, 0xFFFF);
    }
}
