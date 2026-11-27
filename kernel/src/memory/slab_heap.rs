use core::alloc::Layout;

use crate::memory::free_list_allocator::{BlockOwner, FreeListAllocator};

pub(crate) const PAGE: usize = 4096;
pub(crate) const ARENA_PAGES: usize = 16;
const SMALL_MAX: usize = 512;
const SPARE_ARENAS: usize = 1;

const CLASS_COUNT: usize = 19;
const CLASS_SIZES: [usize; CLASS_COUNT] = [
    16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512,
];

const SLAB_MAGIC: usize = 0x5AB5_1CE1;
const ARENA_MAGIC: usize = 0xA2E4_A11A;
const BITMAP_WORDS: usize = 12;

#[repr(C)]
struct SlabHeader {
    magic: usize,
    class: usize,
    free_count: usize,
    free_head: usize,
    prev: usize,
    next: usize,
    bitmap: [u32; BITMAP_WORDS],
}

#[repr(C)]
struct ArenaHeader {
    magic: usize,
    next: usize,
    pages_free: usize,
    used_pages: u32,
}

pub struct SlabHeap {
    partial: [usize; CLASS_COUNT],
    arenas: usize,
    bad_free: usize,
}

pub(crate) const fn small_class(layout: &Layout) -> Option<usize> {
    let size = layout.size();
    if size == 0 || size > SMALL_MAX {
        return None;
    }
    let align = layout.align();
    let mut index = 0;
    while index < CLASS_COUNT {
        let class = CLASS_SIZES[index];
        if class >= size && (class & class.wrapping_neg()) >= align {
            return Some(index);
        }
        index += 1;
    }
    None
}

const fn slab_capacity(class: usize) -> usize {
    (PAGE - core::mem::size_of::<SlabHeader>()) / class
}

pub(crate) fn realloc_same_class(layout: &Layout, new_size: usize) -> bool {
    match Layout::from_size_align(new_size, layout.align()) {
        Ok(new_layout) => match (small_class(layout), small_class(&new_layout)) {
            (Some(current), Some(candidate)) => current == candidate,
            _ => false,
        },
        Err(_) => false,
    }
}

const fn slab_header(page: usize) -> usize {
    page + PAGE - core::mem::size_of::<SlabHeader>()
}

impl SlabHeap {
    pub const fn new() -> Self {
        SlabHeap {
            partial: [0; CLASS_COUNT],
            arenas: 0,
            bad_free: 0,
        }
    }

    /// Slots are popped from the class partial list, or from a fresh slab
    /// page grown through Tier 0; every header and free-chain write is
    /// raw. See docs/rationale/kernel-slab-heap.md.
    pub(crate) unsafe fn allocate(
        &mut self,
        tier0: &mut FreeListAllocator,
        layout: Layout,
    ) -> *mut u8 {
        let Some(index) = small_class(&layout) else {
            return core::ptr::null_mut();
        };
        let class = CLASS_SIZES[index];
        let page = if self.partial[index] == 0 {
            match unsafe { self.take_slab_page(tier0, index) } {
                Some(page) => page,
                None => return core::ptr::null_mut(),
            }
        } else {
            self.partial[index]
        };
        let header = slab_header(page) as *mut SlabHeader;
        let slot = unsafe { (*header).free_head };
        if slot == 0 {
            self.bad_free("empty-partial", page);
            return core::ptr::null_mut();
        }
        unsafe {
            (*header).free_head = (slot as *const usize).read();
            (*header).free_count -= 1;
            bitmap_set(header, (slot - page) / class);
        }
        if unsafe { (*header).free_count } == 0 {
            self.unlink_partial(index, page);
        }
        slot as *mut u8
    }

    /// Free routes only by the layout's class and validates magic, class,
    /// slot offset, capacity and the bitmap bit; failures log and leak the
    /// slot. A page that ends up with every slot free is retired back to
    /// its arena. See docs/rationale/kernel-slab-heap.md.
    pub(crate) unsafe fn deallocate(
        &mut self,
        tier0: &mut FreeListAllocator,
        ptr: *mut u8,
        layout: Layout,
    ) {
        let addr = ptr as usize;
        let Some(index) = small_class(&layout) else {
            self.bad_free("wrong-tier", addr);
            return;
        };
        let Some(arena) = self.arena_of(addr) else {
            self.bad_free("outside-arenas", addr);
            return;
        };
        let class = CLASS_SIZES[index];
        let page = addr & !(PAGE - 1);
        let header = slab_header(page) as *mut SlabHeader;
        if unsafe { (*header).magic } != SLAB_MAGIC {
            self.bad_free("bad-magic", addr);
            return;
        }
        if unsafe { (*header).class } != index {
            self.bad_free("wrong-class", addr);
            return;
        }
        let offset = addr - page;
        if offset % class != 0 || offset / class >= slab_capacity(class) {
            self.bad_free("slot-offset", addr);
            return;
        }
        let slot_index = offset / class;
        if unsafe { !bitmap_test(header, slot_index) } {
            self.bad_free("double-free", addr);
            return;
        }
        let was_full = unsafe { (*header).free_count == 0 };
        unsafe {
            bitmap_clear(header, slot_index);
            (addr as *mut usize).write((*header).free_head);
            (*header).free_head = addr;
            (*header).free_count += 1;
        }
        if unsafe { (*header).free_count } == slab_capacity(class) {
            unsafe { self.retire_page(tier0, index, page, arena) };
        } else if was_full {
            self.push_partial(index, page);
        }
    }

    /// Takes a free arena page and writes its slab header and slot chain.
    /// See docs/rationale/kernel-slab-heap.md.
    unsafe fn take_slab_page(
        &mut self,
        tier0: &mut FreeListAllocator,
        index: usize,
    ) -> Option<usize> {
        let class = CLASS_SIZES[index];
        let mut arena = self.arenas;
        loop {
            if arena == 0 {
                arena = unsafe { self.grow_arena(tier0)? };
            }
            let a = arena as *mut ArenaHeader;
            if unsafe { (*a).magic } != ARENA_MAGIC {
                self.bad_free("bad-arena-magic", arena);
                return None;
            }
            if unsafe { (*a).pages_free } > 0 {
                let mut bit = 0;
                while bit < ARENA_PAGES && unsafe { (*a).used_pages } & (1 << bit) != 0 {
                    bit += 1;
                }
                if bit < ARENA_PAGES {
                    unsafe {
                        (*a).used_pages |= 1 << bit;
                        (*a).pages_free -= 1;
                    }
                    let page = arena + bit * PAGE;
                    let header = slab_header(page) as *mut SlabHeader;
                    unsafe {
                        (*header).magic = SLAB_MAGIC;
                        (*header).class = index;
                        (*header).free_count = slab_capacity(class);
                        (*header).prev = 0;
                        (*header).next = 0;
                        let mut head = 0usize;
                        let mut slot = page + (slab_capacity(class) - 1) * class;
                        loop {
                            (slot as *mut usize).write(head);
                            head = slot;
                            if slot == page {
                                break;
                            }
                            slot -= class;
                        }
                        (*header).free_head = head;
                        let mut word = 0;
                        while word < BITMAP_WORDS {
                            (*header).bitmap[word] = 0;
                            word += 1;
                        }
                    }
                    self.push_partial(index, page);
                    return Some(page);
                }
                unsafe {
                    (*a).pages_free = 0;
                }
            }
            arena = unsafe { (*a).next };
        }
    }

    /// Fetches an arena as one page-aligned Tier 0 block and writes its
    /// header; Tier 0 failure is a null return, never a panic. See
    /// docs/rationale/kernel-slab-heap.md.
    unsafe fn grow_arena(&mut self, tier0: &mut FreeListAllocator) -> Option<usize> {
        let layout = Layout::from_size_align(ARENA_PAGES * PAGE, PAGE).ok()?;
        let ptr = unsafe { tier0.allocate(layout, BlockOwner::Kernel) }.ok()?;
        let arena = ptr as usize;
        let header = arena as *mut ArenaHeader;
        unsafe {
            (*header).magic = ARENA_MAGIC;
            (*header).next = self.arenas;
            (*header).pages_free = ARENA_PAGES - 1;
            (*header).used_pages = 1;
        }
        self.arenas = arena;
        Some(arena)
    }

    fn arena_of(&self, addr: usize) -> Option<usize> {
        let mut arena = self.arenas;
        unsafe {
            while arena != 0 {
                if addr >= arena && addr < arena + ARENA_PAGES * PAGE {
                    return Some(arena);
                }
                arena = (*(arena as *const ArenaHeader)).next;
            }
        }
        None
    }

    /// Unlinks a page whose every slot is free, clears its magic so stale
    /// frees land in `bad-magic`, returns the page to its arena, and gives
    /// the arena back to Tier 0 when more than the spare arenas sit empty.
    /// See docs/rationale/kernel-slab-heap.md.
    unsafe fn retire_page(
        &mut self,
        tier0: &mut FreeListAllocator,
        index: usize,
        page: usize,
        arena: usize,
    ) {
        self.unlink_partial(index, page);
        let a = arena as *mut ArenaHeader;
        let bit = (page - arena) / PAGE;
        let empty = unsafe {
            (*(slab_header(page) as *mut SlabHeader)).magic = 0;
            (*a).used_pages &= !(1 << bit);
            (*a).pages_free += 1;
            (*a).pages_free == ARENA_PAGES - 1
        };
        if empty && self.empty_arenas() > SPARE_ARENAS {
            unsafe { self.release_arena(tier0, arena) };
        }
    }

    fn empty_arenas(&self) -> usize {
        let mut count = 0;
        let mut arena = self.arenas;
        unsafe {
            while arena != 0 {
                let a = arena as *const ArenaHeader;
                if (*a).magic == ARENA_MAGIC && (*a).pages_free == ARENA_PAGES - 1 {
                    count += 1;
                }
                arena = (*a).next;
            }
        }
        count
    }

    /// Unlinks the arena and hands its one Tier 0 block back; the caller
    /// proved it holds no slab. See docs/rationale/kernel-slab-heap.md.
    unsafe fn release_arena(&mut self, tier0: &mut FreeListAllocator, arena: usize) {
        unsafe {
            if self.arenas == arena {
                self.arenas = (*(arena as *const ArenaHeader)).next;
            } else {
                let mut prev = self.arenas;
                while prev != 0 && (*(prev as *const ArenaHeader)).next != arena {
                    prev = (*(prev as *const ArenaHeader)).next;
                }
                if prev != 0 {
                    (*(prev as *mut ArenaHeader)).next = (*(arena as *const ArenaHeader)).next;
                }
            }
        }
        unsafe { tier0.deallocate(arena as *mut u8, PAGE) };
    }

    fn push_partial(&mut self, index: usize, page: usize) {
        let head = self.partial[index];
        unsafe {
            let header = slab_header(page) as *mut SlabHeader;
            (*header).prev = 0;
            (*header).next = head;
            if head != 0 {
                (slab_header(head) as *mut SlabHeader).as_mut().unwrap().prev = page;
            }
        }
        self.partial[index] = page;
    }

    fn unlink_partial(&mut self, index: usize, page: usize) {
        unsafe {
            let header = slab_header(page) as *mut SlabHeader;
            let prev = (*header).prev;
            let next = (*header).next;
            if prev == 0 {
                self.partial[index] = next;
            } else {
                (slab_header(prev) as *mut SlabHeader).as_mut().unwrap().next = next;
            }
            if next != 0 {
                (slab_header(next) as *mut SlabHeader).as_mut().unwrap().prev = prev;
            }
            (*header).prev = 0;
            (*header).next = 0;
        }
    }

    fn bad_free(&mut self, reason: &'static str, addr: usize) {
        self.bad_free += 1;
        #[cfg(not(test))]
        if self.bad_free <= 10 {
            crate::kprintln!("BAD-FREE slab {} p=0x{:x}", reason, addr);
        }
        #[cfg(test)]
        let _ = (reason, addr);
    }

    #[cfg(test)]
    fn arena_count(&self) -> usize {
        let mut count = 0;
        let mut arena = self.arenas;
        while arena != 0 {
            count += 1;
            arena = unsafe { (*(arena as *const ArenaHeader)).next };
        }
        count
    }

    #[cfg(test)]
    fn live_slabs(&self, index: usize) -> usize {
        let mut count = 0;
        let mut page = self.partial[index];
        while page != 0 {
            count += 1;
            page = unsafe { (*(slab_header(page) as *const SlabHeader)).next };
        }
        count
    }
}

unsafe fn bitmap_set(header: *mut SlabHeader, index: usize) {
    unsafe {
        *(*header).bitmap.as_mut_ptr().add(index / 32) |= 1u32 << (index % 32);
    }
}

unsafe fn bitmap_clear(header: *mut SlabHeader, index: usize) {
    unsafe {
        *(*header).bitmap.as_mut_ptr().add(index / 32) &= !(1u32 << (index % 32));
    }
}

unsafe fn bitmap_test(header: *const SlabHeader, index: usize) -> bool {
    unsafe { (*header).bitmap[index / 32] & (1u32 << (index % 32)) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::free_list_allocator::{ALLOC_HDR, BACKREF_BYTES};
    use crate::memory::{MAX_MEMORY_BLOCKS, MemoryBlock, MemoryBlocks};
    use core::alloc::Layout;

    const REGION_BYTES: usize = 2 * 1024 * 1024;
    const ARENA_WORST_CHARGE: usize = ARENA_PAGES * PAGE + 4096 + ALLOC_HDR + BACKREF_BYTES;

    struct Fixture {
        tier0: FreeListAllocator,
        heap: SlabHeap,
    }

    impl Fixture {
        fn new(region_bytes: usize) -> Self {
            let region = vec![0u8; region_bytes].leak();
            let base = region.as_mut_ptr() as usize;
            let mut blocks = [MemoryBlock::free(0, 0); MAX_MEMORY_BLOCKS];
            blocks[0] = MemoryBlock::free(base, region_bytes);
            let tier0 = FreeListAllocator::new(&MemoryBlocks { blocks, count: 1 });
            Fixture {
                tier0,
                heap: SlabHeap::new(),
            }
        }

        fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
            let layout = Layout::from_size_align(size, align).unwrap();
            unsafe { self.heap.allocate(&mut self.tier0, layout) }
        }

        fn dealloc(&mut self, ptr: *mut u8, size: usize, align: usize) {
            let layout = Layout::from_size_align(size, align).unwrap();
            unsafe { self.heap.deallocate(&mut self.tier0, ptr, layout) };
        }
    }

    #[test]
    fn new_slab_heap_allocates_nothing() {
        let fixture = Fixture::new(4096);
        assert_eq!(fixture.heap.arenas, 0);
        assert_eq!(fixture.heap.arena_count(), 0);
    }

    #[test]
    fn class_mapping_boundaries() {
        let cases: [(usize, Option<usize>); 14] = [
            (0, None),
            (1, Some(0)),
            (16, Some(0)),
            (17, Some(1)),
            (24, Some(1)),
            (25, Some(2)),
            (33, Some(3)),
            (57, Some(6)),
            (65, Some(7)),
            (511, Some(18)),
            (512, Some(18)),
            (513, None),
            (1024, None),
            (4096, None),
        ];
        for (size, expected) in cases {
            let layout = Layout::from_size_align(size, 8).unwrap();
            assert_eq!(small_class(&layout), expected, "size {}", size);
        }
    }

    #[test]
    fn alignment_rule_picks_a_wider_class_or_falls_back() {
        let cases: [(usize, usize, Option<usize>); 8] = [
            (1, 16, Some(0)),
            (24, 16, Some(2)),
            (40, 16, Some(4)),
            (100, 64, Some(10)),
            (512, 512, Some(18)),
            (16, 1024, None),
            (512, 4096, None),
            (1024, 16, None),
        ];
        for (size, align, expected) in cases {
            let layout = Layout::from_size_align(size, align).unwrap();
            assert_eq!(
                small_class(&layout),
                expected,
                "size {} align {}",
                size,
                align
            );
        }
    }

    #[test]
    fn slot_reuse_is_lifo() {
        let mut fixture = Fixture::new(REGION_BYTES);
        let a = fixture.alloc(32, 8);
        let b = fixture.alloc(32, 8);
        assert!(!a.is_null() && !b.is_null() && a != b);
        fixture.dealloc(b, 32, 8);
        let c = fixture.alloc(32, 8);
        assert_eq!(c, b);
        fixture.dealloc(a, 32, 8);
        fixture.dealloc(c, 32, 8);
    }

    #[test]
    fn slots_carry_the_class_alignment() {
        let mut fixture = Fixture::new(REGION_BYTES);
        for (size, align) in [(16usize, 8usize), (24, 16), (100, 64), (512, 512)] {
            let ptr = fixture.alloc(size, align);
            assert!(!ptr.is_null());
            assert_eq!(ptr as usize % align, 0, "size {} align {}", size, align);
            fixture.dealloc(ptr, size, align);
        }
    }

    #[test]
    fn a_slab_fills_and_the_next_slot_spills_to_a_fresh_page() {
        let mut fixture = Fixture::new(REGION_BYTES);
        let capacity = slab_capacity(64);
        let mut pages = Vec::new();
        for _ in 0..capacity + 1 {
            let ptr = fixture.alloc(64, 8);
            assert!(!ptr.is_null());
            let page = ptr as usize & !(PAGE - 1);
            if !pages.contains(&page) {
                pages.push(page);
            }
        }
        assert_eq!(pages.len(), 2);
        assert_eq!(fixture.heap.live_slabs(6), 1);
    }

    #[test]
    fn a_full_page_leaves_the_partial_list_and_returns_on_free() {
        let mut fixture = Fixture::new(REGION_BYTES);
        let capacity = slab_capacity(512);
        let mut slots = Vec::new();
        for _ in 0..capacity {
            slots.push(fixture.alloc(512, 8));
        }
        assert_eq!(fixture.heap.live_slabs(18), 0);
        let extra = fixture.alloc(512, 8);
        assert!(!extra.is_null());
        assert_eq!(fixture.heap.live_slabs(18), 1);
        fixture.dealloc(slots[0], 512, 8);
        assert_eq!(fixture.heap.live_slabs(18), 2);
        let reuse = fixture.alloc(512, 8);
        assert_eq!(reuse, slots[0]);
    }

    #[test]
    fn arena_growth_then_tier0_exhaustion_returns_null() {
        let region = ARENA_PAGES * PAGE + 2 * PAGE + 512;
        let mut fixture = Fixture::new(region);
        let slots_per_arena = slab_capacity(512) * (ARENA_PAGES - 1);
        let mut slots = Vec::new();
        for _ in 0..slots_per_arena {
            let ptr = fixture.alloc(512, 8);
            assert!(!ptr.is_null());
            slots.push(ptr);
        }
        assert_eq!(fixture.heap.arena_count(), 1);
        assert!(fixture.alloc(512, 8).is_null());
        assert_eq!(fixture.heap.arena_count(), 1);
        assert_eq!(fixture.heap.bad_free, 0);
        for slot in slots {
            fixture.dealloc(slot, 512, 8);
        }
        assert!(!fixture.alloc(512, 8).is_null());
    }

    #[test]
    fn two_arenas_grow_and_fill_before_exhaustion() {
        let region = 2 * ARENA_WORST_CHARGE;
        let mut fixture = Fixture::new(region);
        let slots_per_arena = slab_capacity(512) * (ARENA_PAGES - 1);
        let mut slots = Vec::new();
        for _ in 0..2 * slots_per_arena {
            let ptr = fixture.alloc(512, 8);
            assert!(!ptr.is_null());
            slots.push(ptr);
        }
        assert_eq!(fixture.heap.arena_count(), 2);
        assert!(fixture.alloc(512, 8).is_null());
    }

    #[test]
    fn a_retired_page_returns_to_the_arena_and_is_reused() {
        let mut fixture = Fixture::new(REGION_BYTES);
        let capacity = slab_capacity(512);
        let mut slots = Vec::new();
        for _ in 0..capacity {
            slots.push(fixture.alloc(512, 8));
        }
        let page = slots[0] as usize & !(PAGE - 1);
        let spill = fixture.alloc(512, 8);
        assert_ne!(spill as usize & !(PAGE - 1), page);
        for slot in slots {
            fixture.dealloc(slot, 512, 8);
        }
        fixture.dealloc(spill, 512, 8);
        let free_before = fixture.tier0.stats().free_bytes;
        let reuse = fixture.alloc(512, 8);
        assert_eq!(reuse as usize & !(PAGE - 1), page);
        assert_eq!(
            fixture.tier0.stats().free_bytes,
            free_before,
            "the reused page comes from the arena, not from Tier 0"
        );
        fixture.dealloc(reuse, 512, 8);
        assert_eq!(fixture.heap.bad_free, 0);
    }

    #[test]
    fn empty_arenas_shrink_to_the_single_spare() {
        let region = 2 * ARENA_WORST_CHARGE;
        let mut fixture = Fixture::new(region);
        let slots_per_arena = slab_capacity(512) * (ARENA_PAGES - 1);
        let mut slots = Vec::new();
        for _ in 0..2 * slots_per_arena {
            slots.push(fixture.alloc(512, 8));
        }
        assert_eq!(fixture.heap.arena_count(), 2);
        for slot in slots {
            fixture.dealloc(slot, 512, 8);
        }
        assert_eq!(fixture.heap.arena_count(), SPARE_ARENAS);
        assert_eq!(fixture.heap.bad_free, 0);
        let free_before = fixture.tier0.stats().free_bytes;
        let slot = fixture.alloc(512, 8);
        assert!(!slot.is_null());
        assert_eq!(fixture.heap.arena_count(), SPARE_ARENAS);
        assert_eq!(
            fixture.tier0.stats().free_bytes,
            free_before,
            "the spare arena serves the request without a Tier 0 fetch"
        );
    }

    #[test]
    fn a_stale_free_into_a_released_arena_is_rejected() {
        let region = 2 * ARENA_WORST_CHARGE;
        let mut fixture = Fixture::new(region);
        let slots_per_arena = slab_capacity(512) * (ARENA_PAGES - 1);
        let mut slots = Vec::new();
        for _ in 0..2 * slots_per_arena {
            slots.push(fixture.alloc(512, 8));
        }
        let stale = slots[slots_per_arena];
        for slot in slots {
            fixture.dealloc(slot, 512, 8);
        }
        assert_eq!(fixture.heap.arena_count(), SPARE_ARENAS);
        fixture.dealloc(stale, 512, 8);
        assert_eq!(fixture.heap.bad_free, 1);
    }

    #[test]
    fn double_free_is_rejected() {
        let mut fixture = Fixture::new(REGION_BYTES);
        let ptr = fixture.alloc(32, 8);
        fixture.dealloc(ptr, 32, 8);
        fixture.dealloc(ptr, 32, 8);
        assert_eq!(fixture.heap.bad_free, 1);
        let next = fixture.alloc(32, 8);
        assert!(!next.is_null());
        fixture.dealloc(next, 32, 8);
    }

    #[test]
    fn wrong_class_free_is_rejected() {
        let mut fixture = Fixture::new(REGION_BYTES);
        let ptr = fixture.alloc(24, 8);
        fixture.dealloc(ptr, 16, 8);
        assert_eq!(fixture.heap.bad_free, 1);
        fixture.dealloc(ptr, 24, 8);
        assert_eq!(fixture.heap.bad_free, 1);
    }

    #[test]
    fn misaligned_free_inside_a_slot_is_rejected() {
        let mut fixture = Fixture::new(REGION_BYTES);
        let ptr = fixture.alloc(16, 8);
        let inner = (ptr as usize + 4) as *mut u8;
        fixture.dealloc(inner, 16, 8);
        assert_eq!(fixture.heap.bad_free, 1);
        fixture.dealloc(ptr, 16, 8);
    }

    #[test]
    fn zero_size_requests_never_reach_the_slab() {
        let layout = Layout::from_size_align(0, 8).unwrap();
        assert_eq!(small_class(&layout), None);
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
        fn below(&mut self, n: usize) -> usize {
            (self.next() as usize) % n
        }
    }

    #[test]
    fn randomized_model_checks_overlap_alignment_content_and_reclaim() {
        let mut fixture = Fixture::new(REGION_BYTES);
        let baseline_free = fixture.tier0.stats().free_bytes;
        let mut rng = XorShift(0xDEAD_BEEF_0BAD_F00D);
        let mut live: Vec<(*mut u8, usize, u8)> = Vec::new();
        let mut peak_arenas = 0usize;
        for op in 0..20_000usize {
            if !live.is_empty() && rng.below(100) < 45 {
                let idx = rng.below(live.len());
                let (ptr, size, pattern) = live.swap_remove(idx);
                let slice = unsafe { core::slice::from_raw_parts(ptr, size) };
                assert!(
                    slice.iter().all(|byte| *byte == pattern),
                    "content of a live block was corrupted at op {}",
                    op
                );
                fixture.dealloc(ptr, size, 8);
            } else {
                let size = 1 + rng.below(SMALL_MAX);
                let index = small_class(&Layout::from_size_align(size, 8).unwrap()).unwrap();
                let ptr = fixture.alloc(size, 8);
                assert!(!ptr.is_null());
                let class = CLASS_SIZES[index];
                assert_eq!(ptr as usize % (class & class.wrapping_neg()), 0);
                let pattern = (op % 251) as u8;
                unsafe { core::ptr::write_bytes(ptr, pattern, size) };
                live.push((ptr, size, pattern));
                peak_arenas = peak_arenas.max(fixture.heap.arena_count());
            }
        }
        let mut ordered: Vec<(usize, usize)> = live
            .iter()
            .map(|(ptr, size, _)| (*ptr as usize, *size))
            .collect();
        ordered.sort_unstable();
        for window in ordered.windows(2) {
            assert!(window[0].0 + window[0].1 <= window[1].0);
        }
        for (ptr, size, pattern) in &live {
            let slice = unsafe { core::slice::from_raw_parts(*ptr, *size) };
            assert!(slice.iter().all(|byte| *byte == *pattern));
        }
        assert!(live.len() > 500);
        for (ptr, size, _) in live {
            fixture.dealloc(ptr, size, 8);
        }
        assert!(peak_arenas >= 2);
        assert_eq!(fixture.heap.arena_count(), SPARE_ARENAS);
        let charge_min = ARENA_PAGES * PAGE + ALLOC_HDR + BACKREF_BYTES;
        let charge_max = charge_min + 4 * core::mem::size_of::<usize>();
        let retained = fixture.tier0.stats().free_bytes;
        assert!(
            retained >= baseline_free - charge_max && retained <= baseline_free - charge_min,
            "retained {} baseline {}",
            retained,
            baseline_free
        );
        assert_eq!(fixture.heap.bad_free, 0);
        for index in 0..CLASS_COUNT {
            assert_eq!(fixture.heap.partial[index], 0, "no page stays on a class list");
        }
    }
}
