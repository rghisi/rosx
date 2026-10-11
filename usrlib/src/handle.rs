use core::fmt::{Debug, Display, Formatter};
use system::error::is_token;

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
}
