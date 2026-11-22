use crate::handle::{Handle, next_generation};
use alloc::alloc::{alloc, Layout};
use alloc::boxed::Box;
use alloc::vec::Vec;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    NotFound,
    OutOfMemory,
}

pub(crate) const ARENA_MAX_SLOTS: usize = 1024;

pub(crate) const PAGE_SLOTS: usize = 32;
const LINK_NONE: u16 = u16::MAX;

struct Slot<T> {
    generation: u16,
    next_free: u16,
    item: Option<T>,
}

pub struct GenerationalArena<T> {
    pages: Vec<Box<[Slot<T>]>>,
    slots: usize,
    max_slots: usize,
    free_head: u16,
    free_tail: u16,
}

impl<T> GenerationalArena<T> {
    pub fn new(max_slots: usize) -> Self {
        assert!(max_slots >= 1, "arena cap must be at least one slot");
        assert!(max_slots <= 65535, "arena cap exceeds the 16-bit index space");
        let mut arena = Self {
            pages: Vec::new(),
            slots: 0,
            max_slots,
            free_head: LINK_NONE,
            free_tail: LINK_NONE,
        };
        arena.grow().expect("the first arena page must allocate");
        arena
    }

    /// The page comes from the raw allocator so a full kernel heap surfaces
    /// as `OutOfMemory` instead of aborting through the global error handler;
    /// the boxed slice frees it with the same layout, and every slot is
    /// written before the block is reachable.
    /// See docs/rationale/arena-page-allocation.md.
    fn allocate_page(count: usize) -> Option<Box<[Slot<T>]>> {
        let layout = Layout::array::<Slot<T>>(count).ok()?;
        unsafe {
            let block = alloc(layout).cast::<Slot<T>>();
            if block.is_null() {
                return None;
            }
            for offset in 0..count {
                block.add(offset).write(Slot { generation: 0, next_free: LINK_NONE, item: None });
            }
            Some(Box::from_raw(core::slice::from_raw_parts_mut(block, count)))
        }
    }

    fn grow(&mut self) -> Result<(), Error> {
        let fresh = (self.max_slots - self.slots).min(PAGE_SLOTS);
        if fresh == 0 {
            return Err(Error::OutOfMemory);
        }
        let mut page = Self::allocate_page(fresh).ok_or(Error::OutOfMemory)?;
        let first = self.slots as u16;
        let last = (self.slots + fresh - 1) as u16;
        for offset in 0..fresh as u16 {
            let slot = &mut page[offset as usize];
            slot.next_free = if offset + 1 < fresh as u16 { first + offset + 1 } else { LINK_NONE };
        }
        if self.free_head == LINK_NONE {
            self.free_head = first;
        } else {
            let (tail_page, tail_slot) = Self::address(self.free_tail);
            self.pages[tail_page][tail_slot].next_free = first;
        }
        self.free_tail = last;
        self.pages.push(page);
        self.slots += fresh;
        Ok(())
    }

    const fn address(index: u16) -> (usize, usize) {
        let slot = index as usize;
        (slot / PAGE_SLOTS, slot % PAGE_SLOTS)
    }

    fn slot(&self, index: u16) -> Option<&Slot<T>> {
        if index as usize >= self.slots {
            return None;
        }
        let (page, offset) = Self::address(index);
        Some(&self.pages[page][offset])
    }

    fn slot_mut(&mut self, index: u16) -> Option<&mut Slot<T>> {
        if index as usize >= self.slots {
            return None;
        }
        let (page, offset) = Self::address(index);
        Some(&mut self.pages[page][offset])
    }

    pub(crate) fn add(&mut self, item: T) -> Result<Handle, Error> {
        if self.free_head == LINK_NONE {
            self.grow()?;
        }
        let index = self.free_head;
        let (page, offset) = Self::address(index);
        let slot = &mut self.pages[page][offset];
        self.free_head = slot.next_free;
        if self.free_head == LINK_NONE {
            self.free_tail = LINK_NONE;
        }
        let generation = slot.generation;
        slot.item = Some(item);
        Ok(Handle::new(index, generation))
    }

    pub(crate) fn borrow(&self, handle: Handle) -> Result<&T, Error> {
        let slot = self.slot(handle.index).ok_or(Error::NotFound)?;
        if slot.generation == handle.generation {
            if let Some(item) = &slot.item {
                return Ok(item);
            }
        }
        Err(Error::NotFound)
    }

    pub(crate) fn borrow_mut(&mut self, handle: Handle) -> Result<&mut T, Error> {
        let slot = self.slot_mut(handle.index).ok_or(Error::NotFound)?;
        if slot.generation == handle.generation {
            if let Some(item) = &mut slot.item {
                return Ok(item);
            }
        }
        Err(Error::NotFound)
    }

    pub(crate) fn remove(&mut self, handle: Handle) -> Result<T, Error> {
        let slot = self.slot_mut(handle.index).ok_or(Error::NotFound)?;
        if slot.generation != handle.generation {
            return Err(Error::NotFound);
        }
        let item = slot.item.take().ok_or(Error::NotFound)?;
        slot.generation = next_generation(slot.generation);
        slot.next_free = LINK_NONE;
        if self.free_head == LINK_NONE {
            self.free_head = handle.index;
        } else {
            let (tail_page, tail_slot) = Self::address(self.free_tail);
            self.pages[tail_page][tail_slot].next_free = handle.index;
        }
        self.free_tail = handle.index;
        Ok(item)
    }

    pub(crate) fn live_generation(&self, index: u16) -> Option<u16> {
        let slot = self.slot(index)?;
        slot.item.as_ref()?;
        Some(slot.generation)
    }

    pub(crate) fn any<F: Fn(&T) -> bool>(&self, predicate: F) -> bool {
        self.pages.iter().flatten().any(|slot| slot.item.as_ref().is_some_and(&predicate))
    }

    pub(crate) fn visit<F: FnMut(&T)>(&self, mut visitor: F) {
        for slot in self.pages.iter().flatten() {
            if let Some(item) = &slot.item {
                visitor(item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use crate::collections::arena::{Error, GenerationalArena, PAGE_SLOTS};
    use std::string::String;
    use std::string::ToString;
    use std::vec;

    #[test]
    fn starts_with_one_page_of_slots() {
        let arena: GenerationalArena<i32> = GenerationalArena::new(1024);
        assert_eq!(arena.pages.len(), 1);
        assert_eq!(arena.slots, PAGE_SLOTS);
        assert_eq!(arena.free_head, 0);
        assert_eq!(arena.free_tail, (PAGE_SLOTS - 1) as u16);
    }

    #[test]
    fn fills_the_first_page_without_growing() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(1024);
        for slot in 0..PAGE_SLOTS {
            let handle = arena.add(slot as i32).unwrap();
            assert_eq!(handle.index, slot as u16);
        }
        assert_eq!(arena.pages.len(), 1);
    }

    #[test]
    fn grows_across_page_boundaries() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(100);
        let mut handles = vec![];
        for slot in 0..100 {
            handles.push(arena.add(slot).unwrap());
        }
        assert_eq!(arena.pages.len(), 4);
        assert_eq!(arena.slots, 100);
        assert_eq!(arena.pages[3].len(), 4);
        for (slot, handle) in handles.iter().enumerate() {
            assert_eq!(*arena.borrow(*handle).unwrap(), slot as i32);
        }
        assert_eq!(arena.add(101), Err(Error::OutOfMemory));
    }

    #[test]
    fn handles_stay_valid_across_growth() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(1024);
        let early = arena.add(7).unwrap();
        for slot in 0..200 {
            arena.add(slot).unwrap();
        }
        assert_eq!(*arena.borrow(early).unwrap(), 7);
    }

    #[test]
    fn reuses_freed_slots_in_fifo_order() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(64);
        let a = arena.add(1).unwrap();
        let b = arena.add(2).unwrap();
        let c = arena.add(3).unwrap();
        arena.remove(b).unwrap();
        arena.remove(a).unwrap();
        arena.remove(c).unwrap();
        for _ in 3..PAGE_SLOTS as u16 {
            arena.add(0).unwrap();
        }
        assert_eq!(arena.add(4).unwrap().index, b.index);
        assert_eq!(arena.add(5).unwrap().index, a.index);
        assert_eq!(arena.add(6).unwrap().index, c.index);
        assert_eq!(arena.add(7).unwrap().index, PAGE_SLOTS as u16);
    }

    #[test]
    fn churn_below_page_size_never_grows() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(64);
        let mut live = vec![];
        for round in 0..200i32 {
            let handle = arena.add(round).unwrap();
            assert!(handle.index < PAGE_SLOTS as u16);
            live.push(handle);
            if live.len() > 16 {
                arena.remove(live.remove(0)).unwrap();
            }
        }
        assert_eq!(arena.pages.len(), 1);
    }

    #[test]
    fn grows_only_when_the_free_list_drains() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(64);
        let mut handles = vec![];
        for slot in 0..64 {
            handles.push(arena.add(slot).unwrap());
        }
        assert_eq!(arena.pages.len(), 2);
        let doomed = handles[40];
        arena.remove(doomed).unwrap();
        let recycled = arena.add(65).unwrap();
        assert_eq!(recycled.index, doomed.index);
        assert_eq!(recycled.generation, doomed.generation + 1);
        assert_eq!(arena.pages.len(), 2);
        assert_eq!(arena.add(66), Err(Error::OutOfMemory));
    }

    #[test]
    fn out_of_memory_at_cap_never_panics_on_growth_failure() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(40);
        for slot in 0..40 {
            arena.add(slot).unwrap();
        }
        assert_eq!(arena.add(41), Err(Error::OutOfMemory));
    }

    #[test]
    fn borrowed_mut_changes_the_stored_item() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(64);
        let handle = arena.add(10).unwrap();
        *arena.borrow_mut(handle).unwrap() = 20;
        assert_eq!(*arena.borrow(handle).unwrap(), 20);
    }

    #[test]
    fn stale_handles_are_rejected_after_remove() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(64);
        let old = arena.add(42).unwrap();
        arena.remove(old).unwrap();
        assert_eq!(arena.borrow(old), Err(Error::NotFound));
        assert_eq!(arena.borrow_mut(old), Err(Error::NotFound));
        assert_eq!(arena.remove(old), Err(Error::NotFound));
    }

    #[test]
    fn stale_handles_are_rejected_after_the_slot_is_reused() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(1);
        let old = arena.add(1).unwrap();
        arena.remove(old).unwrap();
        let fresh = arena.add(2).unwrap();
        assert_eq!(fresh.index, old.index);
        assert_eq!(arena.borrow(old), Err(Error::NotFound));
        assert_eq!(*arena.borrow(fresh).unwrap(), 2);
    }

    #[test]
    fn handles_to_never_materialized_slots_are_rejected() {
        let arena: GenerationalArena<i32> = GenerationalArena::new(64);
        let ghost = arena.borrow(crate::handle::Handle::new(50, 0));
        assert_eq!(ghost, Err(Error::NotFound));
    }

    #[test]
    fn live_generation_reports_only_occupied_slots_and_advances_on_remove() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(64);
        assert_eq!(arena.live_generation(0), None);
        let handle = arena.add(7).unwrap();
        assert_eq!(arena.live_generation(handle.index), Some(handle.generation));
        arena.remove(handle).unwrap();
        assert_eq!(arena.live_generation(handle.index), None);
        let recycled = arena.add(9).unwrap();
        assert_eq!(arena.live_generation(recycled.index), Some(recycled.generation));
        assert_eq!(arena.live_generation(60), None);
    }

    #[test]
    fn visit_yields_every_live_item_once_across_pages() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(128);
        for slot in 0..100 {
            arena.add(slot).unwrap();
        }
        let (mut sum, mut count) = (0i64, 0usize);
        arena.visit(|item| {
            sum += *item as i64;
            count += 1;
        });
        assert_eq!((sum, count), (4950, 100));
    }

    #[test]
    fn visit_skips_removed_slots() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(128);
        let first = arena.add(10).unwrap();
        arena.add(20).unwrap();
        arena.remove(first).unwrap();
        let mut seen = 0usize;
        arena.visit(|_| seen += 1);
        assert_eq!(seen, 1);
    }

    #[test]
    fn any_finds_only_live_items_across_pages() {
        let mut arena: GenerationalArena<i32> = GenerationalArena::new(128);
        for slot in 0..100 {
            arena.add(slot).unwrap();
        }
        assert!(arena.any(|item| *item == 99));
        assert!(!arena.any(|item| *item == 100));
    }

    #[test]
    fn complex_type_storage() {
        #[derive(Debug, PartialEq)]
        struct ComplexType {
            id: u32,
            name: String,
            values: Vec<i32>,
        }
        let mut arena: GenerationalArena<ComplexType> = GenerationalArena::new(64);
        let item = ComplexType { id: 42, name: "test".to_string(), values: vec![1, 2, 3] };
        let handle = arena.add(item).unwrap();
        let retrieved = arena.borrow(handle).unwrap();
        assert_eq!(retrieved.id, 42);
        assert_eq!(retrieved.name, "test");
        assert_eq!(retrieved.values, vec![1, 2, 3]);
    }

    #[test]
    #[should_panic(expected = "arena cap must be at least one slot")]
    fn panics_on_zero_cap() {
        let _arena: GenerationalArena<i32> = GenerationalArena::new(0);
    }

    #[test]
    #[should_panic(expected = "arena cap exceeds the 16-bit index space")]
    fn panics_on_cap_above_the_16_bit_index_space() {
        let _arena: GenerationalArena<i32> = GenerationalArena::new(65536);
    }
}
