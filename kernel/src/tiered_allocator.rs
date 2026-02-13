use core::mem::size_of;
use core::ptr::null_mut;

pub trait BlockProvider {
    fn block_size(&self) -> usize;
    fn allocate_blocks(&self, min_blocks: usize) -> Option<(*mut u8, usize)>;
    fn release_blocks(&self, ptr: *mut u8, count: usize);
}

const ALIGNMENT: usize = size_of::<usize>() * 2;
const MIN_CHUNK_SIZE: usize = size_of::<usize>() * 4;

#[repr(C)]
struct RegionHeader {
    total_size: usize,
    block_count: usize,
    next: *mut RegionHeader,
    free_list_head: *mut FreeChunk,
    used_bytes: usize,
}

#[repr(C)]
struct FreeChunk {
    size_and_flags: usize,
    next: *mut FreeChunk,
    prev: *mut FreeChunk,
}

struct ChunkHeader {
    size_and_flags: usize,
}

impl ChunkHeader {
    fn size(&self) -> usize {
        self.size_and_flags & !1
    }

    fn is_allocated(&self) -> bool {
        self.size_and_flags & 1 != 0
    }
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

fn chunk_footer(chunk: *mut u8, chunk_size: usize) -> *mut usize {
    unsafe { chunk.add(chunk_size - size_of::<usize>()) as *mut usize }
}

unsafe fn init_region(base: *mut u8, total_size: usize, block_count: usize) -> *mut RegionHeader {
    let header = base as *mut RegionHeader;
    let usable_start = align_up(base as usize + size_of::<RegionHeader>(), ALIGNMENT);
    let usable_size = total_size - (usable_start - base as usize);
    let chunk_size = usable_size - (usable_size % ALIGNMENT);

    let chunk = usable_start as *mut FreeChunk;

    unsafe {
        (*chunk).size_and_flags = chunk_size;
        (*chunk).next = null_mut();
        (*chunk).prev = null_mut();

        let footer = chunk_footer(chunk as *mut u8, chunk_size);
        *footer = chunk_size;

        (*header).total_size = total_size;
        (*header).block_count = block_count;
        (*header).next = null_mut();
        (*header).free_list_head = chunk;
        (*header).used_bytes = 0;
    }

    header
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::cell::RefCell;
    use std::vec;
    use std::vec::Vec;

    struct MockBlockProvider {
        block_size: usize,
        memory: Vec<u8>,
        next_offset: RefCell<usize>,
        released: RefCell<Vec<(*mut u8, usize)>>,
    }

    impl MockBlockProvider {
        fn new(block_size: usize, total_blocks: usize) -> Self {
            MockBlockProvider {
                block_size,
                memory: vec![0u8; block_size * total_blocks],
                next_offset: RefCell::new(0),
                released: RefCell::new(Vec::new()),
            }
        }

        fn released_count(&self) -> usize {
            self.released.borrow().len()
        }
    }

    impl BlockProvider for MockBlockProvider {
        fn block_size(&self) -> usize {
            self.block_size
        }

        fn allocate_blocks(&self, min_blocks: usize) -> Option<(*mut u8, usize)> {
            let mut offset = self.next_offset.borrow_mut();
            let bytes_needed = min_blocks * self.block_size;
            if *offset + bytes_needed > self.memory.len() {
                return None;
            }
            let ptr = unsafe { self.memory.as_ptr().add(*offset) as *mut u8 };
            *offset += bytes_needed;
            Some((ptr, min_blocks))
        }

        fn release_blocks(&self, ptr: *mut u8, count: usize) {
            self.released.borrow_mut().push((ptr, count));
        }
    }

    fn init_test_region(provider: &MockBlockProvider, blocks: usize) -> *mut RegionHeader {
        let (ptr, count) = provider.allocate_blocks(blocks).unwrap();
        unsafe { init_region(ptr, count * provider.block_size(), count) }
    }

    #[test]
    fn mock_provider_should_allocate_blocks() {
        let provider = MockBlockProvider::new(4096, 4);
        let result = provider.allocate_blocks(1);
        assert!(result.is_some());
        let (ptr, count) = result.unwrap();
        assert!(!ptr.is_null());
        assert_eq!(count, 1);
    }

    #[test]
    fn mock_provider_should_allocate_multiple_blocks() {
        let provider = MockBlockProvider::new(4096, 4);
        let (ptr1, _) = provider.allocate_blocks(2).unwrap();
        let (ptr2, _) = provider.allocate_blocks(2).unwrap();
        assert_eq!(unsafe { ptr2.offset_from(ptr1) }, 8192);
    }

    #[test]
    fn mock_provider_should_return_none_when_exhausted() {
        let provider = MockBlockProvider::new(4096, 2);
        provider.allocate_blocks(2).unwrap();
        assert!(provider.allocate_blocks(1).is_none());
    }

    #[test]
    fn mock_provider_should_track_releases() {
        let provider = MockBlockProvider::new(4096, 4);
        let (ptr, _) = provider.allocate_blocks(1).unwrap();
        provider.release_blocks(ptr, 1);
        assert_eq!(provider.released_count(), 1);
    }

    #[test]
    fn region_init_should_create_single_free_chunk() {
        let provider = MockBlockProvider::new(4096, 1);
        let region = init_test_region(&provider, 1);

        unsafe {
            assert_eq!((*region).total_size, 4096);
            assert_eq!((*region).block_count, 1);
            assert!((*region).next.is_null());
            assert_eq!((*region).used_bytes, 0);
            assert!(!(*region).free_list_head.is_null());
        }
    }

    #[test]
    fn region_init_should_have_free_chunk_covering_usable_space() {
        let provider = MockBlockProvider::new(4096, 1);
        let region = init_test_region(&provider, 1);

        unsafe {
            let chunk = (*region).free_list_head;
            let chunk_size = (*chunk).size_and_flags & !1;

            let base = region as usize;
            let usable_start = align_up(base + size_of::<RegionHeader>(), ALIGNMENT);
            let expected_size = 4096 - (usable_start - base);
            let expected_size = expected_size - (expected_size % ALIGNMENT);

            assert_eq!(chunk_size, expected_size);
            assert_eq!(chunk as usize, usable_start);
            assert!((*chunk).next.is_null());
            assert!((*chunk).prev.is_null());
        }
    }

    #[test]
    fn region_init_should_write_footer() {
        let provider = MockBlockProvider::new(4096, 1);
        let region = init_test_region(&provider, 1);

        unsafe {
            let chunk = (*region).free_list_head;
            let chunk_size = (*chunk).size_and_flags & !1;
            let footer = chunk_footer(chunk as *mut u8, chunk_size);
            assert_eq!(*footer, chunk_size);
        }
    }

    #[test]
    fn region_init_should_work_with_multiple_blocks() {
        let provider = MockBlockProvider::new(4096, 4);
        let region = init_test_region(&provider, 3);

        unsafe {
            assert_eq!((*region).total_size, 4096 * 3);
            assert_eq!((*region).block_count, 3);

            let chunk = (*region).free_list_head;
            let chunk_size = (*chunk).size_and_flags & !1;
            assert!(chunk_size > 4096 * 2);
        }
    }

    #[test]
    fn region_free_chunk_should_not_be_marked_allocated() {
        let provider = MockBlockProvider::new(4096, 1);
        let region = init_test_region(&provider, 1);

        unsafe {
            let chunk = (*region).free_list_head as *mut ChunkHeader;
            assert!(!(*chunk).is_allocated());
        }
    }
}
