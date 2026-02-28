use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::fmt::Debug;

#[cfg(target_pointer_width = "64")]
pub type HalfSize = u32;
#[cfg(target_pointer_width = "32")]
pub type HalfSize = u16;
#[cfg(target_pointer_width = "16")]
pub type HalfSize = u8;

const HALF_BITS: usize = core::mem::size_of::<HalfSize>() * 8;

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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Handle {
    pub index: HalfSize,
    pub generation: HalfSize,
}

impl Handle {
    pub fn new(index: HalfSize, generation: HalfSize) -> Self {
        Self { index, generation }
    }

    pub fn pack(&self) -> usize {
        ((self.index as usize) << HALF_BITS) | (self.generation as usize)
    }

    pub fn unpack(packed: usize) -> Self {
        let mask = (1usize << HALF_BITS) - 1;
        Self {
            index: (packed >> HALF_BITS) as HalfSize,
            generation: (packed & mask) as HalfSize,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    NotFound,
    OutOfMemory,
}

pub struct GenArena<T, I: IndexType> {
    items: Vec<Option<T>>,
    generations: Vec<HalfSize>,
    free_slots: VecDeque<I>,
}

impl<T, I: IndexType> GenArena<T, I> {
    pub fn new(initial_capacity: usize) -> Self {
        let capacity = I::max_value();
        assert!(
            initial_capacity <= capacity,
            "Initial capacity cannot exceed the max value of the index type"
        );
        assert!(initial_capacity > 0, "Initial capacity cannot be zero");

        let mut items = Vec::with_capacity(initial_capacity);
        let mut generations = Vec::with_capacity(initial_capacity);
        let mut free_slots = VecDeque::with_capacity(initial_capacity);
        for slot in 0..initial_capacity {
            items.push(None);
            generations.push(0);
            free_slots.push_back(I::from_usize(slot));
        }

        Self {
            items,
            generations,
            free_slots,
        }
    }

    pub fn add(&mut self, item: T) -> Result<Handle, Error> {
        if self.free_slots.is_empty() {
            let increment = self.generations.capacity().max(1);
            if increment >= I::max_value() {
                return Err(Error::OutOfMemory);
            }
            let new_size = (increment + increment).min(I::max_value());
            for i in increment..new_size {
                self.items.push(None);
                self.generations.push(0);
                self.free_slots.push_back(I::from_usize(i));
            }
        }

        let index = self.free_slots.pop_front().unwrap();
        let generation = self.generations[index.as_usize()];
        self.items[index.as_usize()] = Some(item);

        Ok(Handle::new(index.as_usize() as HalfSize, generation))
    }

    pub fn borrow(&self, handle: Handle) -> Result<&T, Error> {
        let index = handle.index as usize;
        if index < self.items.len() && self.generations[index] == handle.generation {
            return Ok(self.items[index].as_ref().unwrap());
        }
        Err(Error::NotFound)
    }

    pub fn borrow_mut(&mut self, handle: Handle) -> Result<&mut T, Error> {
        let index = handle.index as usize;
        if index < self.items.len() && self.generations[index] == handle.generation {
            return Ok(self.items[index].as_mut().unwrap());
        }
        Err(Error::NotFound)
    }

    pub fn remove(&mut self, handle: Handle) -> Result<T, Error> {
        let index = handle.index as usize;
        if index >= self.items.len() || self.generations[index] != handle.generation {
            return Err(Error::NotFound);
        }
        let item = self.items[index].take().unwrap();
        self.generations[index] = self.generations[index].wrapping_add(1);
        self.free_slots.push_back(I::from_usize(index));
        Ok(item)
    }

    pub fn replace(&mut self, handle: Handle, item: T) -> Result<Handle, Error> {
        let index = handle.index as usize;
        if index >= self.items.len() || self.generations[index] != handle.generation {
            return Err(Error::NotFound);
        }
        self.items[index] = Some(item);
        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use crate::generational_arena::{Error, GenArena, Handle, HalfSize};
    use std::string::String;
    use std::string::ToString;
    use std::vec;

    #[test]
    fn handle_pack_should_encode_index_in_upper_half_and_generation_in_lower_half() {
        let handle = Handle::new(5 as HalfSize, 10 as HalfSize);
        let half_bits = core::mem::size_of::<HalfSize>() * 8;
        assert_eq!(handle.pack(), (5usize << half_bits) | 10usize);
    }

    #[test]
    fn handle_unpack_should_decode_index_and_generation_from_usize() {
        let half_bits = core::mem::size_of::<HalfSize>() * 8;
        let packed = (5usize << half_bits) | 10usize;
        let handle = Handle::unpack(packed);
        assert_eq!(handle.index, 5 as HalfSize);
        assert_eq!(handle.generation, 10 as HalfSize);
    }

    #[test]
    fn handle_pack_unpack_should_roundtrip() {
        let original = Handle::new(42 as HalfSize, 7 as HalfSize);
        assert_eq!(Handle::unpack(original.pack()), original);
    }

    #[test]
    fn handles_should_be_equals_when_created_with_the_same_index_and_generation() {
        let h1 = Handle::new(5, 10);
        let h2 = Handle::new(5, 10);
        let h3 = Handle::new(5, 11);

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn should_initialize_to_the_initial_capacity() {
        let arena: GenArena<i32, u8> = GenArena::new(10);
        assert_eq!(arena.items.len(), 10);
        assert_eq!(arena.generations.len(), 10);
        assert_eq!(arena.free_slots.len(), 10);
    }

    #[test]
    fn should_borrow_when_handle_is_valid() {
        let mut arena: GenArena<i32, u8> = GenArena::new(5);
        let handle = arena.add(42).unwrap();
        let value = arena.borrow(handle).unwrap();
        assert_eq!(*value, 42);
    }

    #[test]
    fn should_borrow_mut_when_handle_is_valid() {
        let mut arena: GenArena<i32, u8> = GenArena::new(5);
        let handle = arena.add(10).unwrap();

        {
            let value = arena.borrow_mut(handle).unwrap();
            *value = 20;
        }

        assert_eq!(*arena.borrow(handle).unwrap(), 20);
    }

    #[test]
    fn should_accept_multiple_items_when_there_are_free_slots() {
        let mut arena: GenArena<String, u8> = GenArena::new(3);
        let h1 = arena.add("first".to_string()).unwrap();
        let h2 = arena.add("second".to_string()).unwrap();
        let h3 = arena.add("third".to_string()).unwrap();

        assert_eq!(arena.borrow(h1).unwrap(), "first");
        assert_eq!(arena.borrow(h2).unwrap(), "second");
        assert_eq!(arena.borrow(h3).unwrap(), "third");
    }

    #[test]
    fn should_return_error_when_full_and_capacity_cannot_be_extended() {
        let mut arena: GenArena<i32, u8> = GenArena::new(255);

        for i in 0..255 {
            arena.add(i).unwrap();
        }

        assert_eq!(arena.add(999), Err(Error::OutOfMemory));
    }

    #[test]
    fn should_return_error_when_borrowing_removed_handle() {
        let mut arena: GenArena<i32, u8> = GenArena::new(5);
        let old_handle = arena.add(42).unwrap();

        arena.remove(old_handle).unwrap();

        assert_eq!(arena.borrow(old_handle), Err(Error::NotFound));
        assert_eq!(arena.borrow_mut(old_handle), Err(Error::NotFound));
    }

    #[test]
    fn should_return_error_when_removing_invalid_handle() {
        let mut arena: GenArena<i32, u8> = GenArena::new(5);
        let handle = arena.add(42).unwrap();

        arena.remove(handle).unwrap();
        assert_eq!(arena.remove(handle), Err(Error::NotFound));
    }

    #[test]
    fn should_grow_when_out_of_space() {
        let mut arena: GenArena<i32, u16> = GenArena::new(2);

        let h1 = arena.add(1).unwrap();
        let h2 = arena.add(2).unwrap();
        let h3 = arena.add(3).unwrap();

        assert_eq!(*arena.borrow(h1).unwrap(), 1);
        assert_eq!(*arena.borrow(h2).unwrap(), 2);
        assert_eq!(*arena.borrow(h3).unwrap(), 3);
        assert!(arena.items.len() >= 3);
    }

    #[test]
    fn should_double_capacity_when_out_of_space() {
        let mut arena: GenArena<i32, u16> = GenArena::new(4);

        for i in 0..4 {
            arena.add(i).unwrap();
        }

        let capacity_before = arena.items.capacity();
        arena.add(100).unwrap();

        assert_eq!(capacity_before, 4);
        assert_eq!(arena.items.capacity(), 8);
    }

    #[test]
    fn should_maintain_consistency_when_added_and_removed_multiple_times() {
        let mut arena: GenArena<i32, u8> = GenArena::new(3);

        let h1 = arena.add(1).unwrap();
        let h2 = arena.add(2).unwrap();

        arena.remove(h1).unwrap();
        let h3 = arena.add(3).unwrap();

        arena.remove(h2).unwrap();
        let h4 = arena.add(4).unwrap();

        assert_eq!(*arena.borrow(h3).unwrap(), 3);
        assert_eq!(*arena.borrow(h4).unwrap(), 4);
        assert_eq!(arena.borrow(h1), Err(Error::NotFound));
        assert_eq!(arena.borrow(h2), Err(Error::NotFound));
    }

    #[test]
    fn should_allow_complex_type_storage() {
        #[derive(Debug, PartialEq)]
        struct ComplexType {
            id: u32,
            name: String,
            values: Vec<i32>,
        }

        let mut arena: GenArena<ComplexType, u16> = GenArena::new(5);

        let item = ComplexType {
            id: 42,
            name: "test".to_string(),
            values: vec![1, 2, 3],
        };

        let handle = arena.add(item).unwrap();
        let retrieved = arena.borrow(handle).unwrap();

        assert_eq!(retrieved.id, 42);
        assert_eq!(retrieved.name, "test");
        assert_eq!(retrieved.values, vec![1, 2, 3]);
    }

    #[test]
    #[should_panic(expected = "Initial capacity cannot be zero")]
    fn should_panic_when_initial_capacity_is_zero() {
        let _arena: GenArena<i32, u8> = GenArena::new(0);
    }

    #[test]
    #[should_panic(expected = "Initial capacity cannot exceed the max value of the index type")]
    fn should_panic_when_initial_capacity_is_too_large() {
        let _arena: GenArena<i32, u8> = GenArena::new(256);
    }

    #[test]
    fn should_round_hobin_indexes_when_adding_and_removing() {
        let mut arena: GenArena<i32, u8> = GenArena::new(3);

        let h1 = arena.add(1).unwrap();
        arena.remove(h1).unwrap();

        let h2 = arena.add(2).unwrap();
        arena.remove(h2).unwrap();

        let h3 = arena.add(3).unwrap();
        arena.remove(h3).unwrap();

        let h4 = arena.add(4).unwrap();
        arena.remove(h4).unwrap();

        let h5 = arena.add(4).unwrap();
        arena.remove(h5).unwrap();

        let h6 = arena.add(4).unwrap();
        arena.remove(h6).unwrap();

        assert_eq!(h1.index, 0);
        assert_eq!(h2.index, 1);
        assert_eq!(h3.index, 2);
        assert_eq!(h4.index, 0);
        assert_eq!(h5.index, 1);
        assert_eq!(h6.index, 2);
    }
}
