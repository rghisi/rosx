use alloc::collections::VecDeque;
use alloc::vec::Vec;

#[cfg(target_pointer_width = "64")]
pub type HalfSize = u32;
#[cfg(target_pointer_width = "32")]
pub type HalfSize = u16;
#[cfg(target_pointer_width = "16")]
pub type HalfSize = u8;

const HALF_BITS: usize = core::mem::size_of::<HalfSize>() * 8;

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

pub struct GenerationalArena<T, const S: usize> {
    items: Vec<Option<T>>,
    generations: Vec<HalfSize>,
    free_slots: VecDeque<HalfSize>,
}

impl<T, const S: usize> GenerationalArena<T, S> {
    pub fn new() -> Self {
        assert!(S > 0, "S must be greater than zero");
        let mut items = Vec::with_capacity(S);
        let mut generations = Vec::with_capacity(S);
        let mut free_slots = VecDeque::with_capacity(S);
        for slot in 0..S {
            items.push(None);
            generations.push(0);
            free_slots.push_back(slot as HalfSize);
        }
        Self { items, generations, free_slots }
    }

    pub fn add(&mut self, item: T) -> Result<Handle, Error> {
        match self.free_slots.pop_front() {
            Some(index) => {
                let generation = self.generations[index as usize];
                self.items[index as usize] = Some(item);
                Ok(Handle::new(index, generation))
            }
            None => Err(Error::OutOfMemory),
        }
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
        self.free_slots.push_back(handle.index);
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
    use crate::generational_arena::{Error, GenerationalArena, Handle, HalfSize};
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
    fn should_initialize_with_s_slots() {
        let arena: GenerationalArena<i32, 10> = GenerationalArena::new();
        assert_eq!(arena.items.len(), 10);
        assert_eq!(arena.generations.len(), 10);
        assert_eq!(arena.free_slots.len(), 10);
    }

    #[test]
    fn should_borrow_when_handle_is_valid() {
        let mut arena: GenerationalArena<i32, 5> = GenerationalArena::new();
        let handle = arena.add(42).unwrap();
        let value = arena.borrow(handle).unwrap();
        assert_eq!(*value, 42);
    }

    #[test]
    fn should_borrow_mut_when_handle_is_valid() {
        let mut arena: GenerationalArena<i32, 5> = GenerationalArena::new();
        let handle = arena.add(10).unwrap();

        {
            let value = arena.borrow_mut(handle).unwrap();
            *value = 20;
        }

        assert_eq!(*arena.borrow(handle).unwrap(), 20);
    }

    #[test]
    fn should_accept_multiple_items_when_there_are_free_slots() {
        let mut arena: GenerationalArena<String, 3> = GenerationalArena::new();
        let h1 = arena.add("first".to_string()).unwrap();
        let h2 = arena.add("second".to_string()).unwrap();
        let h3 = arena.add("third".to_string()).unwrap();

        assert_eq!(arena.borrow(h1).unwrap(), "first");
        assert_eq!(arena.borrow(h2).unwrap(), "second");
        assert_eq!(arena.borrow(h3).unwrap(), "third");
    }

    #[test]
    fn should_return_error_when_full() {
        let mut arena: GenerationalArena<i32, 3> = GenerationalArena::new();
        arena.add(1).unwrap();
        arena.add(2).unwrap();
        arena.add(3).unwrap();
        assert_eq!(arena.add(4), Err(Error::OutOfMemory));
    }

    #[test]
    fn should_return_error_when_borrowing_removed_handle() {
        let mut arena: GenerationalArena<i32, 5> = GenerationalArena::new();
        let old_handle = arena.add(42).unwrap();

        arena.remove(old_handle).unwrap();

        assert_eq!(arena.borrow(old_handle), Err(Error::NotFound));
        assert_eq!(arena.borrow_mut(old_handle), Err(Error::NotFound));
    }

    #[test]
    fn should_return_error_when_removing_invalid_handle() {
        let mut arena: GenerationalArena<i32, 5> = GenerationalArena::new();
        let handle = arena.add(42).unwrap();

        arena.remove(handle).unwrap();
        assert_eq!(arena.remove(handle), Err(Error::NotFound));
    }

    #[test]
    fn should_maintain_consistency_when_added_and_removed_multiple_times() {
        let mut arena: GenerationalArena<i32, 3> = GenerationalArena::new();

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

        let mut arena: GenerationalArena<ComplexType, 5> = GenerationalArena::new();

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
    #[should_panic(expected = "S must be greater than zero")]
    fn should_panic_when_s_is_zero() {
        let _arena: GenerationalArena<i32, 0> = GenerationalArena::new();
    }

    #[test]
    fn should_round_robin_indexes_when_adding_and_removing() {
        let mut arena: GenerationalArena<i32, 3> = GenerationalArena::new();

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
