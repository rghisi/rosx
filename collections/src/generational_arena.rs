use alloc::collections::VecDeque;
use alloc::vec::Vec;

pub const ERROR_BIT: usize = 1 << 31;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Handle {
    pub index: u16,
    pub generation: u16,
}

impl Handle {
    pub fn new(index: u16, generation: u16) -> Self {
        Self { index, generation }
    }

    pub fn pack(&self) -> usize {
        (((self.generation as usize) & 0x7FFF) << 16) | (self.index as usize)
    }

    pub fn unpack(packed: usize) -> Self {
        Self {
            index: (packed & 0xFFFF) as u16,
            generation: ((packed >> 16) & 0x7FFF) as u16,
        }
    }

    pub const fn is_handle(raw: usize) -> bool {
        raw & !(0x7FFF_FFFFusize) == 0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    NotFound,
    OutOfMemory,
}

pub struct GenerationalArena<T, const S: usize> {
    items: Vec<Option<T>>,
    generations: Vec<u16>,
    free_slots: VecDeque<u16>,
}

impl<T, const S: usize> GenerationalArena<T, S> {
    pub fn new() -> Self {
        assert!(S > 0, "S must be greater than zero");
        assert!(S <= 65536, "arena exceeds 16-bit index space");
        let mut items = Vec::with_capacity(S);
        let mut generations = Vec::with_capacity(S);
        let mut free_slots = VecDeque::with_capacity(S);
        for slot in 0..S {
            items.push(None);
            generations.push(0);
            free_slots.push_back(slot as u16);
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
        self.generations[index] = (self.generations[index] + 1) & 0x7FFF;
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
    use crate::generational_arena::{Error, GenerationalArena, Handle, ERROR_BIT};
    use std::string::String;
    use std::string::ToString;
    use std::vec;

    #[test]
    fn handle_pack_should_encode_generation_in_bits_30_to_16_and_index_in_bits_15_to_0() {
        let handle = Handle::new(5, 10);
        assert_eq!(handle.pack(), (10usize << 16) | 5);
    }

    #[test]
    fn handle_unpack_should_decode_index_and_generation_from_usize() {
        let packed = (10usize << 16) | 5;
        let handle = Handle::unpack(packed);
        assert_eq!(handle.index, 5);
        assert_eq!(handle.generation, 10);
    }

    #[test]
    fn handle_pack_unpack_should_roundtrip() {
        let original = Handle::new(42, 7);
        assert_eq!(Handle::unpack(original.pack()), original);
    }

    #[test]
    fn handle_pack_unpack_should_roundtrip_edge_handles() {
        for (index, generation) in [(0u16, 0u16), (65535, 0), (0, 32767), (65535, 32767)] {
            let handle = Handle::new(index, generation);
            assert_eq!(Handle::unpack(handle.pack()), handle);
        }
    }

    #[test]
    fn handle_pack_should_never_set_error_bit() {
        let mut index = 0u16;
        loop {
            let mut generation = 0u16;
            loop {
                assert_eq!(Handle::new(index, generation).pack() & ERROR_BIT, 0);
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
    fn is_handle_should_accept_only_wellformed_packed_values() {
        assert!(Handle::is_handle(0));
        assert!(Handle::is_handle(0x7FFF_FFFF));
        assert!(!Handle::is_handle(ERROR_BIT));
        assert!(!Handle::is_handle(ERROR_BIT | 1));
        assert!(!Handle::is_handle(1usize << 32));
        assert!(!Handle::is_handle(usize::MAX));
    }

    #[test]
    fn generation_counter_should_cycle_from_32767_to_0_without_setting_error_bit() {
        let mut arena: GenerationalArena<u8, 1> = GenerationalArena::new();
        for counter in 0..32768usize {
            let handle = arena.add(0).unwrap();
            assert_eq!(handle.generation, (counter & 0x7FFF) as u16);
            assert_eq!(handle.pack() & ERROR_BIT, 0);
            arena.remove(handle).unwrap();
        }
        let wrapped = arena.add(0).unwrap();
        assert_eq!(wrapped.generation, 0);
        assert_eq!(wrapped.pack() & ERROR_BIT, 0);
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
