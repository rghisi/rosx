use core::fmt::Debug;
use core::ops::{Add, AddAssign};

pub trait IndexType: Copy + PartialEq + Default {
    fn max_value() -> usize;
    fn from_usize(v: usize) -> Self;
    fn as_usize(&self) -> usize;
}

impl IndexType for u8 {
    fn max_value() -> usize {
        u8::MAX as usize
    }
    fn from_usize(v: usize) -> Self {
        v as Self
    }
    fn as_usize(&self) -> usize {
        *self as usize
    }
}
impl IndexType for u16 {
    fn max_value() -> usize {
        u16::MAX as usize
    }
    fn from_usize(v: usize) -> Self {
        v as Self
    }
    fn as_usize(&self) -> usize {
        *self as usize
    }
}
impl IndexType for u32 {
    fn max_value() -> usize {
        u32::MAX as usize
    }
    fn from_usize(v: usize) -> Self {
        v as Self
    }
    fn as_usize(&self) -> usize {
        *self as usize
    }
}
impl IndexType for usize {
    fn max_value() -> usize {
        usize::MAX
    }
    fn from_usize(v: usize) -> Self {
        v as Self
    }
    fn as_usize(&self) -> usize {
        *self
    }
}

pub trait GenerationType:
    Copy + PartialEq + Eq + Add<Self, Output = Self> + AddAssign + From<u8> + Default + Debug
{
}

impl GenerationType for u8 {}
impl GenerationType for u16 {}
impl GenerationType for u32 {}
impl GenerationType for u64 {}
impl GenerationType for usize {}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Handle<I: IndexType, G: GenerationType> {
    pub index: I,
    pub generation: G,
}

impl<I: IndexType, G: GenerationType> Handle<I, G> {
    pub fn new(index: I, generation: G) -> Self {
        Self { index, generation }
    }
}
