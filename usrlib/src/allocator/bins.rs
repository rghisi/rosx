use core::alloc::Layout;

pub const SLAB_BYTES: usize = 65_536;
pub const SMALL_MAX: usize = 4_096;
const MIN_CHUNK_BYTES: usize = 16;
const CLASS_COUNT: usize = 9;

pub const fn small_class_of(layout: &Layout) -> Option<usize> {
    let size = layout.size();
    if size > SMALL_MAX {
        return None;
    }
    let rounded = size.next_power_of_two();
    let chunk = if rounded < MIN_CHUNK_BYTES {
        MIN_CHUNK_BYTES
    } else {
        rounded
    };
    if layout.align() > chunk {
        return None;
    }
    Some(chunk.trailing_zeros() as usize - MIN_CHUNK_BYTES.trailing_zeros() as usize)
}

pub const fn chunk_size_of_class(class: usize) -> Option<usize> {
    if class < CLASS_COUNT {
        Some(MIN_CHUNK_BYTES << class)
    } else {
        None
    }
}

pub struct Binner {
    heads: [usize; CLASS_COUNT],
}

impl Binner {
    pub const fn new() -> Self {
        Binner { heads: [0; CLASS_COUNT] }
    }

    pub unsafe fn take(&mut self, class: usize) -> Option<usize> {
        let slot = self.heads.get_mut(class)?;
        let head = *slot;
        if head == 0 {
            return None;
        }
        *slot = unsafe { (head as *const usize).read() };
        Some(head)
    }

    pub unsafe fn put(&mut self, class: usize, chunk: usize) {
        let Some(slot) = self.heads.get_mut(class) else {
            return;
        };
        unsafe { (chunk as *mut usize).write(*slot) };
        *slot = chunk;
    }

    pub unsafe fn adopt(&mut self, class: usize, slab_start: usize) {
        if slab_start == 0 || slab_start % SMALL_MAX != 0 {
            return;
        }
        let Some(chunk) = chunk_size_of_class(class) else {
            return;
        };
        let Some(slot) = self.heads.get_mut(class) else {
            return;
        };
        let mut next = *slot;
        let mut offset = SLAB_BYTES - chunk;
        loop {
            let address = slab_start + offset;
            unsafe { (address as *mut usize).write(next) };
            next = address;
            if offset == 0 {
                break;
            }
            offset -= chunk;
        }
        *slot = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    struct FakeSlab {
        _backing: Vec<u8>,
        start: usize,
    }

    impl FakeSlab {
        fn new() -> Self {
            let backing = vec![0u8; SLAB_BYTES + SMALL_MAX];
            let start = (backing.as_ptr() as usize + SMALL_MAX - 1) & !(SMALL_MAX - 1);
            FakeSlab {
                _backing: backing,
                start,
            }
        }
    }

    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    fn layout(size: usize, align: usize) -> Layout {
        Layout::from_size_align(size, align).unwrap()
    }

    fn drain(binner: &mut Binner, class: usize) -> Vec<usize> {
        let mut taken = Vec::new();
        while let Some(chunk) = unsafe { binner.take(class) } {
            taken.push(chunk);
        }
        taken
    }

    #[test]
    fn small_class_maps_each_size_to_the_pow2_ladder() {
        assert_eq!(small_class_of(&layout(1, 1)), Some(0));
        assert_eq!(small_class_of(&layout(15, 8)), Some(0));
        assert_eq!(small_class_of(&layout(16, 8)), Some(0));
        assert_eq!(small_class_of(&layout(17, 8)), Some(1));
        assert_eq!(small_class_of(&layout(24, 8)), Some(1));
        assert_eq!(small_class_of(&layout(33, 8)), Some(2));
        assert_eq!(small_class_of(&layout(1000, 8)), Some(6));
        assert_eq!(small_class_of(&layout(4096, 8)), Some(8));
    }

    #[test]
    fn zero_size_routes_to_the_minimal_class() {
        assert_eq!(small_class_of(&layout(0, 1)), Some(0));
        assert_eq!(small_class_of(&layout(0, 16)), Some(0));
    }

    #[test]
    fn requests_above_small_max_route_to_the_large_path() {
        assert_eq!(small_class_of(&layout(4097, 8)), None);
        assert_eq!(small_class_of(&layout(1 << 20, 8)), None);
    }

    #[test]
    fn alignment_beyond_the_chunk_routes_to_the_large_path() {
        assert_eq!(small_class_of(&layout(16, 32)), None);
        assert_eq!(small_class_of(&layout(64, 4096)), None);
        assert_eq!(small_class_of(&layout(32, 32)), Some(1));
        assert_eq!(small_class_of(&layout(4096, 4096)), Some(8));
    }

    #[test]
    fn each_exact_class_size_routes_to_its_own_class() {
        for class in 0..CLASS_COUNT {
            let chunk = chunk_size_of_class(class).unwrap();
            assert_eq!(small_class_of(&layout(chunk, 8)), Some(class));
            if chunk > MIN_CHUNK_BYTES {
                assert_eq!(small_class_of(&layout(chunk - 1, 8)), Some(class));
            }
        }
    }

    #[test]
    fn chunk_sizes_cover_the_nine_class_ladder() {
        let sizes: Vec<usize> = (0..CLASS_COUNT)
            .map(|class| chunk_size_of_class(class).unwrap())
            .collect();
        assert_eq!(sizes, vec![16, 32, 64, 128, 256, 512, 1024, 2048, 4096]);
        assert_eq!(chunk_size_of_class(CLASS_COUNT), None);
    }

    #[test]
    fn take_from_empty_bin_returns_none() {
        let mut binner = Binner::new();
        for class in 0..CLASS_COUNT {
            assert_eq!(unsafe { binner.take(class) }, None);
        }
    }

    #[test]
    fn take_out_of_range_class_returns_none_and_put_ignores_it() {
        let mut binner = Binner::new();
        assert_eq!(unsafe { binner.take(CLASS_COUNT) }, None);
        unsafe { binner.put(CLASS_COUNT, 0x1000) };
        assert_eq!(unsafe { binner.take(CLASS_COUNT) }, None);
    }

    #[test]
    fn adopted_slab_carves_exactly_its_chunk_count_all_aligned_and_unique() {
        for class in 0..CLASS_COUNT {
            let slab = FakeSlab::new();
            let mut binner = Binner::new();
            unsafe { binner.adopt(class, slab.start) };
            let chunk = chunk_size_of_class(class).unwrap();
            let taken = drain(&mut binner, class);
            assert_eq!(taken.len(), SLAB_BYTES / chunk, "class {class}");
            let mut unique = HashSet::new();
            for address in taken {
                assert!(address >= slab.start && address < slab.start + SLAB_BYTES);
                assert_eq!(address % chunk, 0, "class {class}");
                assert!(address % MIN_CHUNK_BYTES == 0);
                assert!(unique.insert(address), "double handout in class {class}");
            }
            drop(slab);
        }
    }

    #[test]
    fn take_returns_the_fresh_slab_in_address_order_then_frees_land_lifo() {
        let slab = FakeSlab::new();
        let mut binner = Binner::new();
        unsafe { binner.adopt(0, slab.start) };
        let first = unsafe { binner.take(0) }.unwrap();
        let second = unsafe { binner.take(0) }.unwrap();
        assert_eq!(first, slab.start);
        assert_eq!(second, slab.start + MIN_CHUNK_BYTES);
        unsafe { binner.put(0, second) };
        unsafe { binner.put(0, first) };
        assert_eq!(unsafe { binner.take(0) }, Some(first));
        assert_eq!(unsafe { binner.take(0) }, Some(second));
    }

    #[test]
    fn drained_bin_refills_from_a_second_adopted_slab() {
        let first = FakeSlab::new();
        let second = FakeSlab::new();
        let mut binner = Binner::new();
        unsafe { binner.adopt(3, first.start) };
        let first_drain = drain(&mut binner, 3);
        assert_eq!(unsafe { binner.take(3) }, None);
        unsafe { binner.adopt(3, second.start) };
        let mut second_drain = drain(&mut binner, 3);
        second_drain.extend(drain(&mut binner, 3));
        assert_eq!(first_drain.len() + second_drain.len(), 2 * SLAB_BYTES / 128);
    }

    #[test]
    fn adopt_chains_onto_chunks_already_in_the_bin() {
        let first = FakeSlab::new();
        let second = FakeSlab::new();
        let mut binner = Binner::new();
        unsafe { binner.adopt(2, first.start) };
        let held = unsafe { binner.take(2) }.unwrap();
        unsafe { binner.adopt(2, second.start) };
        let mut all = drain(&mut binner, 2);
        unsafe { binner.put(2, held) };
        all.extend(drain(&mut binner, 2));
        assert_eq!(all.len(), 2 * SLAB_BYTES / 64);
        let unique: HashSet<usize> = all.into_iter().collect();
        assert_eq!(unique.len(), 2 * SLAB_BYTES / 64);
    }

    #[test]
    fn adopt_ignores_null_and_misaligned_slabs() {
        let mut binner = Binner::new();
        unsafe { binner.adopt(0, 0) };
        unsafe { binner.adopt(0, 4096 + 8) };
        assert_eq!(unsafe { binner.take(0) }, None);
        let slab = FakeSlab::new();
        unsafe { binner.adopt(0, slab.start + MIN_CHUNK_BYTES) };
        assert_eq!(unsafe { binner.take(0) }, None);
    }

    #[test]
    fn classes_keep_independent_stacks() {
        let small_slab = FakeSlab::new();
        let large_slab = FakeSlab::new();
        let mut binner = Binner::new();
        unsafe { binner.adopt(0, small_slab.start) };
        unsafe { binner.adopt(4, large_slab.start) };
        let small = unsafe { binner.take(0) }.unwrap();
        let large = unsafe { binner.take(4) }.unwrap();
        assert!(small >= small_slab.start && small < small_slab.start + SLAB_BYTES);
        assert!(large >= large_slab.start && large < large_slab.start + SLAB_BYTES);
        assert_eq!(drain(&mut binner, 0).len(), SLAB_BYTES / 16 - 1);
        assert_eq!(drain(&mut binner, 4).len(), SLAB_BYTES / 256 - 1);
        assert_eq!(unsafe { binner.take(1) }, None);
    }

    #[test]
    fn freeing_every_other_chunk_returns_exactly_those_to_the_bin() {
        let slab = FakeSlab::new();
        let mut binner = Binner::new();
        unsafe { binner.adopt(1, slab.start) };
        let mut held = Vec::new();
        while let Some(chunk) = unsafe { binner.take(1) } {
            held.push(chunk);
        }
        let freed: Vec<usize> = held.iter().step_by(2).copied().collect();
        for chunk in &freed {
            unsafe { binner.put(1, *chunk) };
        }
        let returned: HashSet<usize> = drain(&mut binner, 1).into_iter().collect();
        assert_eq!(returned, freed.iter().copied().collect::<HashSet<_>>());
        let still_held: HashSet<usize> =
            held.iter().skip(1).step_by(2).copied().collect();
        assert_eq!(still_held.len(), SLAB_BYTES / 32 - freed.len());
        assert_eq!(returned.intersection(&still_held).count(), 0);
    }

    #[test]
    fn full_round_trip_returns_the_whole_slab_to_the_bin() {
        let slab = FakeSlab::new();
        let mut binner = Binner::new();
        unsafe { binner.adopt(6, slab.start) };
        let mut held = drain(&mut binner, 6);
        assert_eq!(held.len(), SLAB_BYTES / 1024);
        for chunk in &held {
            unsafe { binner.put(6, *chunk) };
        }
        let mut returned = drain(&mut binner, 6);
        held.sort_unstable();
        returned.sort_unstable();
        assert_eq!(returned, held);
    }

    #[test]
    fn deterministic_mixed_sequence_never_hands_out_a_chunk_twice() {
        let mut binner = Binner::new();
        let mut slabs = Vec::new();
        let mut slab_starts = Vec::new();
        for class in 0..CLASS_COUNT {
            let slab = FakeSlab::new();
            slab_starts.push(slab.start);
            unsafe { binner.adopt(class, slab.start) };
            slabs.push(slab);
        }
        let mut live: HashMap<usize, usize> = HashMap::new();
        let mut held: Vec<(usize, usize)> = Vec::new();
        let mut random = XorShift(0x2545F4914F6CDD1D);
        for _ in 0..20_000 {
            let roll = random.next();
            let class = (roll % CLASS_COUNT as u64) as usize;
            if roll & 2 == 0 {
                if let Some(chunk) = unsafe { binner.take(class) } {
                    let start = slab_starts[class];
                    assert!(chunk >= start && chunk < start + SLAB_BYTES);
                    assert_eq!(chunk % chunk_size_of_class(class).unwrap(), 0);
                    assert!(live.insert(chunk, class).is_none(), "double handout");
                    held.push((chunk, class));
                }
            } else if !held.is_empty() {
                let index = (random.next() % held.len() as u64) as usize;
                let (chunk, chunk_class) = held.swap_remove(index);
                assert_eq!(live.remove(&chunk), Some(chunk_class));
                unsafe { binner.put(chunk_class, chunk) };
            }
        }
        for class in 0..CLASS_COUNT {
            let expected = SLAB_BYTES / chunk_size_of_class(class).unwrap();
            let live_of_class: HashSet<usize> = live
                .iter()
                .filter(|(_, owner)| **owner == class)
                .map(|(address, _)| *address)
                .collect();
            let drained: Vec<usize> = drain(&mut binner, class);
            let unique_drained: HashSet<usize> = drained.iter().copied().collect();
            assert_eq!(unique_drained.len(), drained.len(), "bin duplication in class {class}");
            assert_eq!(unique_drained.intersection(&live_of_class).count(), 0);
            assert_eq!(unique_drained.len() + live_of_class.len(), expected, "class {class}");
        }
        assert_eq!(held.len(), live.len());
    }
}
