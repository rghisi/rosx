use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::fmt::Debug;

#[cfg(target_pointer_width = "64")]
pub type HalfSize = u32;
#[cfg(target_pointer_width = "32")]
pub type HalfSize = u16;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Handle {
    pub index: HalfSize,
    pub generation: HalfSize,
}

impl Handle {
    pub fn new(index: HalfSize, generation: HalfSize) -> Self {
        Self { index, generation }
    }

    pub fn to_usize(&self) -> usize {
        let index = self.index as usize;
        let generation = self.generation as usize;
        let shift = (core::mem::size_of::<usize>() * 8) / 2;
        (index << shift) | generation
    }

    pub fn from_usize(val: usize) -> Self {
        let shift = (core::mem::size_of::<usize>() * 8) / 2;
        let mask = (1 << shift) - 1;
        let index = (val >> shift) as HalfSize;
        let generation = (val & mask) as HalfSize;
        Self { index, generation }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    NotFound,
    OutOfMemory,
}

pub struct GenArena<T, const S: usize> {
    items: Vec<Option<T>>,
    generations: Vec<HalfSize>,
    free_slots: VecDeque<HalfSize>,
}

impl<T, const S: usize> GenArena<T, S> {
    pub fn new() -> Self {
        assert!(S > 0, "Initial capacity cannot be zero");
        assert!(
            S <= HalfSize::MAX as usize,
            "Initial capacity cannot exceed the max value of the index type"
        );

        let mut items = Vec::with_capacity(S);
        let mut generations = Vec::with_capacity(S);
        let mut free_slots = VecDeque::with_capacity(S);
        for slot in 0..S {
            items.push(None);
            generations.push(0);
            free_slots.push_back(slot as HalfSize);
        }

        Self {
            items,
            generations,
            free_slots,
        }
    }

    pub fn add(&mut self, item: T) -> Result<Handle, Error> {
        if self.free_slots.is_empty() {
            return Err(Error::OutOfMemory);
        }

        let index = self.free_slots.pop_front().unwrap();
        let generation = self.generations[index as usize];
        self.items[index as usize] = Some(item);

        Ok(Handle::new(index, generation))
    }

    pub fn borrow(&self, handle: Handle) -> Result<&T, Error> {
        if (handle.index as usize) < self.generations.len()
            && self.generations[handle.index as usize] == handle.generation
        {
            return Ok(self.items[handle.index as usize].as_ref().unwrap());
        }
        Err(Error::NotFound)
    }

    pub fn borrow_mut(&mut self, handle: Handle) -> Result<&mut T, Error> {
        if (handle.index as usize) < self.generations.len()
            && self.generations[handle.index as usize] == handle.generation
        {
            return Ok(self.items[handle.index as usize].as_mut().unwrap());
        }
        Err(Error::NotFound)
    }

    pub fn remove(&mut self, handle: Handle) -> Result<T, Error> {
        let index = handle.index as usize;
        let generation = handle.generation;
        if index < self.generations.len() && generation == self.generations[index] {
            let item = self.items[index].take().unwrap();
            self.generations[index] = self.generations[index].wrapping_add(1);
            self.free_slots.push_back(handle.index);
            Ok(item)
        } else {
            Err(Error::NotFound)
        }
    }

    pub fn replace(&mut self, handle: Handle, item: T) -> Result<Handle, Error> {
        let index = handle.index as usize;
        let generation = handle.generation;
        if index < self.generations.len() && generation == self.generations[index] {
            self.items[index] = Some(item);
            Ok(handle)
        } else {
            Err(Error::NotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use crate::generational_arena::{Error, GenArena, Handle};
    use std::string::String;
    use std::string::ToString;
    use std::vec;

    #[test]
    fn should_initialize_to_the_initial_capacity() {
        let arena: GenArena<i32, 10> = GenArena::new();
        assert_eq!(arena.items.len(), 10);
        assert_eq!(arena.generations.len(), 10);
        assert_eq!(arena.free_slots.len(), 10);
    }

    #[test]
    fn should_borrow_when_handle_is_valid() {
        let mut arena: GenArena<i32, 5> = GenArena::new();
        let handle = arena.add(42).unwrap();
        let value = arena.borrow(handle).unwrap();
        assert_eq!(*value, 42);
    }

    #[test]
    fn should_borrow_mut_when_handle_is_valid() {
        let mut arena: GenArena<i32, 5> = GenArena::new();
        let handle = arena.add(10).unwrap();

        {
            let value = arena.borrow_mut(handle).unwrap();
            *value = 20;
        }

        assert_eq!(*arena.borrow(handle).unwrap(), 20);
    }

    #[test]
    fn should_accept_multiple_items_when_there_are_free_slots() {
        let mut arena: GenArena<String, 3> = GenArena::new();
        let h1 = arena.add("first".to_string()).unwrap();
        let h2 = arena.add("second".to_string()).unwrap();
        let h3 = arena.add("third".to_string()).unwrap();

        assert_eq!(arena.borrow(h1).unwrap(), "first");
        assert_eq!(arena.borrow(h2).unwrap(), "second");
        assert_eq!(arena.borrow(h3).unwrap(), "third");
    }

    #[test]
    fn should_return_error_when_full() {
        let mut arena: GenArena<i32, 2> = GenArena::new();

        arena.add(0).unwrap();
        arena.add(1).unwrap();

        assert_eq!(arena.add(999), Err(Error::OutOfMemory));
    }

    #[test]
    fn should_return_error_when_borrowing_removed_handle() {
        let mut arena: GenArena<i32, 5> = GenArena::new();
        let old_handle = arena.add(42).unwrap();

        arena.remove(old_handle).unwrap();

        assert_eq!(arena.borrow(old_handle), Err(Error::NotFound));
        assert_eq!(arena.borrow_mut(old_handle), Err(Error::NotFound));
    }

    #[test]
    fn should_return_error_when_removing_invalid_handle() {
        let mut arena: GenArena<i32, 5> = GenArena::new();
        let handle = arena.add(42).unwrap();

        arena.remove(handle).unwrap();
        assert_eq!(arena.remove(handle), Err(Error::NotFound));
    }

    #[test]
    fn should_maintain_consistency_when_added_and_removed_multiple_times() {
        let mut arena: GenArena<i32, 3> = GenArena::new();

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
    fn handles_should_be_equals_when_created_with_the_same_index_and_generation() {
        let h1 = Handle::new(5, 10);
        let h2 = Handle::new(5, 10);
        let h3 = Handle::new(5, 11);

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn should_allow_complex_type_storage() {
        #[derive(Debug, PartialEq)]
        struct ComplexType {
            id: u32,
            name: String,
            values: Vec<i32>,
        }

        let mut arena: GenArena<ComplexType, 5> = GenArena::new();

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
        let _arena: GenArena<i32, 0> = GenArena::new();
    }

    #[test]
    fn should_round_hobin_indexes_when_adding_and_removing() {
        let mut arena: GenArena<i32, 3> = GenArena::new();

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

    #[test]
    fn should_pack_and_unpack_handle() {
        let h = Handle::new(0x12345678, 0x9ABCDEF0);
        let packed = h.to_usize();

        #[cfg(target_pointer_width = "64")]
        assert_eq!(packed, 0x123456789ABCDEF0);

        let unpacked = Handle::from_usize(packed);
        assert_eq!(h, unpacked);
    }
}
