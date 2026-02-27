use alloc::collections::VecDeque;
use alloc::vec::Vec;

#[cfg(target_pointer_width = "64")]
pub type HalfWord = u32;
#[cfg(target_pointer_width = "32")]
pub type HalfWord = u16;
#[cfg(target_pointer_width = "16")]
pub type HalfWord = u8;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Handle {
    pub index: HalfWord,
    pub generation: HalfWord,
}

impl Handle {
    pub fn new(index: HalfWord, generation: HalfWord) -> Self {
        Self { index, generation }
    }

    pub fn pack(&self) -> usize {
        ((self.index as usize) << (usize::BITS / 2)) | (self.generation as usize)
    }

    pub fn unpack(v: usize) -> Self {
        Self {
            index: (v >> (usize::BITS / 2)) as HalfWord,
            generation: v as HalfWord,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    NotFound,
    OutOfMemory,
}

pub struct GenArena<T, const N: usize> {
    items: Vec<Option<T>>,
    generations: Vec<HalfWord>,
    free_slots: VecDeque<HalfWord>,
}

impl<T, const N: usize> GenArena<T, N> {
    const VALID_CAPACITY: () = assert!(N > 0 && N <= HalfWord::MAX as usize + 1);

    pub fn new() -> Self {
        #[allow(clippy::let_unit_value)]
        let _ = Self::VALID_CAPACITY;

        let mut items = Vec::with_capacity(N);
        let mut generations = Vec::with_capacity(N);
        let mut free_slots = VecDeque::with_capacity(N);
        for slot in 0..N {
            items.push(None);
            generations.push(0);
            free_slots.push_back(slot as HalfWord);
        }

        Self {
            items,
            generations,
            free_slots,
        }
    }

    pub fn add(&mut self, item: T) -> Result<Handle, Error> {
        let index = self.free_slots.pop_front().ok_or(Error::OutOfMemory)?;
        let generation = self.generations[index as usize];
        self.items[index as usize] = Some(item);
        Ok(Handle::new(index, generation))
    }

    pub fn borrow(&self, handle: Handle) -> Result<&T, Error> {
        if self.generations[handle.index as usize] == handle.generation {
            return Ok(self.items[handle.index as usize].as_ref().unwrap());
        }
        Err(Error::NotFound)
    }

    pub fn borrow_mut(&mut self, handle: Handle) -> Result<&mut T, Error> {
        if self.generations[handle.index as usize] == handle.generation {
            return Ok(self.items[handle.index as usize].as_mut().unwrap());
        }
        Err(Error::NotFound)
    }

    pub fn remove(&mut self, handle: Handle) -> Result<T, Error> {
        let index = handle.index as usize;
        if handle.generation == self.generations[index] {
            let item = self.items[index].take().unwrap();
            self.generations[index] += 1;
            self.free_slots.push_back(handle.index);
            Ok(item)
        } else {
            Err(Error::NotFound)
        }
    }

    pub fn replace(&mut self, handle: Handle, item: T) -> Result<Handle, Error> {
        let index = handle.index as usize;
        if handle.generation == self.generations[index] {
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
    use crate::generational_arena::{Error, GenArena, Handle, HalfWord};
    use std::string::String;
    use std::string::ToString;
    use std::vec;

    #[test]
    fn should_initialize_with_n_slots() {
        let arena: GenArena<i32, 10> = GenArena::new();
        assert_eq!(arena.items.len(), 10);
        assert_eq!(arena.generations.len(), 10);
        assert_eq!(arena.free_slots.len(), 10);
    }

    #[test]
    fn should_borrow_when_handle_is_valid() {
        let mut arena: GenArena<i32, 5> = GenArena::new();
        let handle = arena.add(42).unwrap();
        assert_eq!(*arena.borrow(handle).unwrap(), 42);
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
    fn should_return_out_of_memory_when_full() {
        let mut arena: GenArena<i32, 3> = GenArena::new();
        arena.add(1).unwrap();
        arena.add(2).unwrap();
        arena.add(3).unwrap();
        assert_eq!(arena.add(4), Err(Error::OutOfMemory));
    }

    #[test]
    fn should_allow_add_after_remove_when_full() {
        let mut arena: GenArena<i32, 2> = GenArena::new();
        let h1 = arena.add(1).unwrap();
        arena.add(2).unwrap();
        arena.remove(h1).unwrap();
        assert!(arena.add(3).is_ok());
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
    fn should_replace_item_keeping_same_handle() {
        let mut arena: GenArena<i32, 5> = GenArena::new();
        let handle = arena.add(42).unwrap();
        let new_handle = arena.replace(handle, 100).unwrap();
        assert_eq!(handle, new_handle);
        assert_eq!(*arena.borrow(handle).unwrap(), 100);
    }

    #[test]
    fn should_return_not_found_when_replacing_with_stale_handle() {
        let mut arena: GenArena<i32, 5> = GenArena::new();
        let handle = arena.add(42).unwrap();
        arena.remove(handle).unwrap();
        assert_eq!(arena.replace(handle, 100), Err(Error::NotFound));
    }

    #[test]
    fn should_allow_complex_type_storage() {
        #[derive(Debug, PartialEq)]
        struct ComplexType {
            id: u32,
            name: String,
            values: std::vec::Vec<i32>,
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
    fn pack_unpack_should_roundtrip() {
        let handle = Handle::new(0xAB as HalfWord, 0xCD as HalfWord);
        assert_eq!(Handle::unpack(handle.pack()), handle);
    }

    #[test]
    fn pack_should_place_index_in_upper_bits_and_generation_in_lower_bits() {
        let handle = Handle::new(1 as HalfWord, 2 as HalfWord);
        let packed = handle.pack();
        assert_eq!(packed >> (usize::BITS / 2), 1);
        assert_eq!(packed & ((1usize << (usize::BITS / 2)) - 1), 2);
    }

    #[test]
    fn handles_should_be_equal_when_created_with_same_index_and_generation() {
        let h1 = Handle::new(5 as HalfWord, 10 as HalfWord);
        let h2 = Handle::new(5 as HalfWord, 10 as HalfWord);
        let h3 = Handle::new(5 as HalfWord, 11 as HalfWord);

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn should_round_robin_indexes_when_adding_and_removing() {
        let mut arena: GenArena<i32, 3> = GenArena::new();

        let h1 = arena.add(1).unwrap();
        arena.remove(h1).unwrap();

        let h2 = arena.add(2).unwrap();
        arena.remove(h2).unwrap();

        let h3 = arena.add(3).unwrap();
        arena.remove(h3).unwrap();

        let h4 = arena.add(4).unwrap();
        arena.remove(h4).unwrap();

        let h5 = arena.add(5).unwrap();
        arena.remove(h5).unwrap();

        let h6 = arena.add(6).unwrap();
        arena.remove(h6).unwrap();

        assert_eq!(h1.index, 0 as HalfWord);
        assert_eq!(h2.index, 1 as HalfWord);
        assert_eq!(h3.index, 2 as HalfWord);
        assert_eq!(h4.index, 0 as HalfWord);
        assert_eq!(h5.index, 1 as HalfWord);
        assert_eq!(h6.index, 2 as HalfWord);
    }
}
