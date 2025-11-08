use alloc::collections::VecDeque;
use alloc::vec::Vec;
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

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    NotFound,
    OutOfMemory,
}

pub struct GenArena<T, I: IndexType, G: GenerationType> {
    items: Vec<Option<T>>,
    generations: Vec<G>,
    free_slots: VecDeque<I>,
}

impl<T, I: IndexType, G: GenerationType> GenArena<T, I, G> {
    pub fn new(initial_capacity: usize) -> Self {
        let capacity = I::max_value();
        assert!(
            initial_capacity <= capacity,
            "Initial capacity cannot exceed the max value of the index type"
        );

        let mut items = Vec::with_capacity(initial_capacity);
        let mut generations = Vec::with_capacity(initial_capacity);
        let mut free_slots = VecDeque::with_capacity(initial_capacity);
        for slot in 0..initial_capacity {
            items.push(None);
            generations.push(G::default());
            free_slots.push_back(I::from_usize(slot));
        }

        Self {
            items,
            generations,
            free_slots,
        }
    }

    pub fn add(&mut self, item: T) -> Result<Handle<I, G>, Error> {
        if self.free_slots.is_empty() {
            let increment = self.generations.capacity();
            if increment >= I::max_value() {
                return Err(Error::OutOfMemory);
            }
            let new_size = (increment + increment).min(I::max_value());
            for i in increment..new_size {
                self.items.push(None);
                self.generations.push(G::default());
                self.free_slots.push_back(I::from_usize(i));
            }
        }

        let index = self.free_slots.pop_front().unwrap();
        let generation = self.generations[index.as_usize()];
        self.items[index.as_usize()] = Some(item);

        Ok(Handle::new(index, generation))
    }

    pub fn borrow(&self, handle: Handle<I, G>) -> Result<&T, Error> {
        if self.generations[handle.index.as_usize()] == handle.generation {
            return Ok(self.items[handle.index.as_usize()].as_ref().unwrap());
        }
        Err(Error::NotFound)
    }

    pub fn borrow_mut(&mut self, handle: Handle<I, G>) -> Result<&mut T, Error> {
        if self.generations[handle.index.as_usize()] == handle.generation {
            return Ok(self.items[handle.index.as_usize()].as_mut().unwrap());
        }
        Err(Error::NotFound)
    }

    pub fn remove(&mut self, handle: Handle<I, G>) -> Result<T, Error> {
        let index = handle.index.as_usize();
        let generation = handle.generation;
        if generation == self.generations[index] {
            let item = self.items[index].take().unwrap();
            self.generations[index] += 1.into();
            self.free_slots.push_back(handle.index);
            Ok(item)
        } else {
            Err(Error::NotFound)
        }
    }
}
