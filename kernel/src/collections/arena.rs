use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::handle::{Handle, next_generation};

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

    pub(crate) fn add(&mut self, item: T) -> Result<Handle, Error> {
        match self.free_slots.pop_front() {
            Some(index) => {
                let generation = self.generations[index as usize];
                self.items[index as usize] = Some(item);
                Ok(Handle::new(index, generation))
            }
            None => Err(Error::OutOfMemory),
        }
    }

    pub(crate) fn borrow(&self, handle: Handle) -> Result<&T, Error> {
        let index = handle.index as usize;
        if index < self.items.len() && self.generations[index] == handle.generation {
            return Ok(self.items[index].as_ref().unwrap());
        }
        Err(Error::NotFound)
    }

    pub(crate) fn borrow_mut(&mut self, handle: Handle) -> Result<&mut T, Error> {
        let index = handle.index as usize;
        if index < self.items.len() && self.generations[index] == handle.generation {
            return Ok(self.items[index].as_mut().unwrap());
        }
        Err(Error::NotFound)
    }

    pub(crate) fn remove(&mut self, handle: Handle) -> Result<T, Error> {
        let index = handle.index as usize;
        if index >= self.items.len() || self.generations[index] != handle.generation {
            return Err(Error::NotFound);
        }
        let item = self.items[index].take().unwrap();
        self.generations[index] = next_generation(self.generations[index]);
        self.free_slots.push_back(handle.index);
        Ok(item)
    }

    pub(crate) fn replace(&mut self, handle: Handle, item: T) -> Result<Handle, Error> {
        let index = handle.index as usize;
        if index >= self.items.len() || self.generations[index] != handle.generation {
            return Err(Error::NotFound);
        }
        self.items[index] = Some(item);
        Ok(handle)
    }

    pub(crate) fn any<F: Fn(&T) -> bool>(&self, predicate: F) -> bool {
        self.items.iter().flatten().any(|item| predicate(item))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use crate::collections::arena::{Error, GenerationalArena};
    use std::string::String;
    use std::string::ToString;
    use std::vec;

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

    #[test]
    fn should_any_match_nothing_when_arena_is_empty() {
        let arena: GenerationalArena<i32, 4> = GenerationalArena::new();
        assert!(!arena.any(|item| *item > 0));
    }

    #[test]
    fn should_any_find_only_occupied_slots() {
        let mut arena: GenerationalArena<i32, 4> = GenerationalArena::new();
        let h1 = arena.add(7).unwrap();
        let h2 = arena.add(9).unwrap();
        assert!(arena.any(|item| *item == 7));
        assert!(arena.any(|item| *item == 9));
        assert!(!arena.any(|item| *item == 8));
        arena.remove(h1).unwrap();
        arena.remove(h2).unwrap();
        assert!(!arena.any(|item| *item == 7));
    }
}
