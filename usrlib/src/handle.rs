use crate::arch;
use crate::error::DataError;
use core::fmt::{Debug, Display, Formatter};
use system::error::{ErrorCode, is_token};
use system::syscall_numbers::SyscallNum;

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct UserFutureHandle {
    packed: u32,
}

impl UserFutureHandle {
    pub(crate) fn packed(self) -> u32 {
        self.packed
    }

    pub(crate) fn from_token(raw: usize) -> Option<Self> {
        if is_token(raw) {
            Some(Self { packed: raw as u32 })
        } else {
            None
        }
    }
}

impl Debug for UserFutureHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:08x}", self.packed)
    }
}

impl Display for UserFutureHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:08x}", self.packed)
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct UserConnectionHandle {
    packed: u32,
}

impl UserConnectionHandle {
    pub(crate) fn packed(self) -> u32 {
        self.packed
    }

    pub(crate) fn from_token(raw: usize) -> Option<Self> {
        if is_token(raw) {
            Some(Self { packed: raw as u32 })
        } else {
            None
        }
    }
}

impl Debug for UserConnectionHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:08x}", self.packed)
    }
}

impl Display for UserConnectionHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:08x}", self.packed)
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct UserBindingHandle {
    packed: u32,
}

impl UserBindingHandle {
    pub(crate) fn packed(self) -> u32 {
        self.packed
    }

    pub(crate) fn from_token(raw: usize) -> Option<Self> {
        if is_token(raw) {
            Some(Self { packed: raw as u32 })
        } else {
            None
        }
    }
}

impl Debug for UserBindingHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:08x}", self.packed)
    }
}

impl Display for UserBindingHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:08x}", self.packed)
    }
}

pub struct UserDataHandle {
    packed: Option<u32>,
}

impl UserDataHandle {
    pub(crate) fn packed(&self) -> u32 {
        match self.packed {
            Some(token) => token,
            None => 0,
        }
    }

    pub fn from_token(raw: usize) -> Option<Self> {
        if is_token(raw) {
            Some(Self {
                packed: Some(raw as u32),
            })
        } else {
            None
        }
    }

    pub fn release(&mut self) -> Result<(), DataError> {
        let Some(raw) = self.packed.take() else {
            return Ok(());
        };
        let result =
            arch::raw_syscall(SyscallNum::IpcDataRelease as usize, raw as usize, 0, 0);
        if is_token(result) {
            Ok(())
        } else {
            Err(DataError::from(ErrorCode::from_code(result)))
        }
    }
}

impl Drop for UserDataHandle {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

impl Debug for UserDataHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.packed {
            Some(packed) => write!(f, "{:08x}", packed),
            None => write!(f, "released"),
        }
    }
}

impl Display for UserDataHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self.packed {
            Some(packed) => write!(f, "{:08x}", packed),
            None => write!(f, "released"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_failure_sentinel_decodes_to_none() {
        assert!(UserFutureHandle::from_token(u64::MAX as usize).is_none());
    }

    #[test]
    fn high_bit_garbage_decodes_to_none_for_every_kind() {
        assert!(UserFutureHandle::from_token(0x8000_0000usize).is_none());
        assert!(UserFutureHandle::from_token(0x8000_0001usize).is_none());
        assert!(UserConnectionHandle::from_token(0x8000_0000usize).is_none());
        assert!(UserConnectionHandle::from_token(0x8000_0001usize).is_none());
        assert!(UserBindingHandle::from_token(0x8000_0000usize).is_none());
        assert!(UserBindingHandle::from_token(0x8000_0001usize).is_none());
        assert!(UserFutureHandle::from_token(usize::MAX).is_none());
        assert!(UserConnectionHandle::from_token(usize::MAX).is_none());
        assert!(UserBindingHandle::from_token(usize::MAX).is_none());
    }

    #[test]
    fn token_words_below_the_high_bit_decode_to_some() {
        assert!(UserFutureHandle::from_token(0).is_some());
        assert!(UserConnectionHandle::from_token(0).is_some());
        assert!(UserBindingHandle::from_token(0).is_some());
        assert!(UserFutureHandle::from_token(0x7FFF_FFFFusize).is_some());
        assert!(UserConnectionHandle::from_token(0x7FFF_FFFFusize).is_some());
        assert!(UserBindingHandle::from_token(0x7FFF_FFFFusize).is_some());
    }

    #[test]
    fn debug_and_display_render_packed_hex_only() {
        let rendered = format!(
            "{:?}",
            UserConnectionHandle::from_token(0x0012_3456usize).unwrap()
        );
        assert_eq!(rendered, "00123456");
        let displayed = format!(
            "{}",
            UserConnectionHandle::from_token(0x0012_3456usize).unwrap()
        );
        assert_eq!(displayed, "00123456");
    }

    #[test]
    fn data_handle_release_on_released_token_is_ok_and_idempotent() {
        let mut handle = UserDataHandle { packed: None };
        assert_eq!(handle.release(), Ok(()));
        assert_eq!(handle.release(), Ok(()));
    }

    #[test]
    fn released_data_handle_renders_as_released_and_packs_to_zero() {
        let mut handle = UserDataHandle { packed: None };
        assert_eq!(format!("{:?}", handle), "released");
        assert_eq!(format!("{}", handle), "released");
        assert_eq!(handle.packed(), 0);
        assert_eq!(handle.release(), Ok(()));
    }

    #[test]
    fn data_handle_from_token_rejects_high_bit_garbage() {
        assert!(UserDataHandle::from_token(0x8000_0000usize).is_none());
        assert!(UserDataHandle::from_token(0x8000_0001usize).is_none());
        assert!(UserDataHandle::from_token(usize::MAX).is_none());
    }
}
