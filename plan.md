# Bitmap Chunk Allocator - Development Plan

Standalone bitmap-based memory allocator that manages memory in fixed 64KB chunks.
Supports multiple memory regions and contiguous multi-chunk allocations.
Developed in isolation — not wired into the kernel's allocator infrastructure yet.

## Design

**Struct:** `BitmapChunkAllocator` in `kernel/src/bitmap_chunk_allocator.rs`

**Key decisions:**
- Chunk size: 64KB (const)
- Bitmap stored as a fixed-size array of `u64` words (each bit = one 64KB chunk)
- Regions that aren't aligned to 64KB boundaries get trimmed (only full chunks are usable)
- Max bitmap size: 2048 `u64` words = 131,072 bits = 131,072 chunks = 8GB addressable. Reasonable starting point.
- Max regions: reuse existing `MAX_MEMORY_BLOCKS` (32)

**Data layout:**
```
BitmapChunkAllocator {
    regions: [Region; 32]       // base addr + chunk count + bitmap offset per region
    region_count: usize
    bitmap: [u64; 2048]         // shared bitmap, each region owns a slice
    total_chunks: usize         // total managed chunks across all regions
}
```

**API:**
- `new()` — create empty allocator
- `add_region(base: usize, size: usize)` — register a memory region (trims to chunk alignment)
- `allocate(chunk_count: usize) -> Option<*mut u8>` — find contiguous free chunks, mark used, return base pointer
- `deallocate(ptr: *mut u8, chunk_count: usize)` — mark chunks as free
- `free_chunks() -> usize` — count of unallocated chunks
- `used_chunks() -> usize` — count of allocated chunks

## Steps (incremental, one at a time)

### Step 1: Create the module with struct and `new()` / `add_region()`
- Create `kernel/src/bitmap_chunk_allocator.rs`
- Define constants (`CHUNK_SIZE`, `MAX_BITMAP_WORDS`, `MAX_REGIONS`)
- Define `Region` (internal) and `BitmapChunkAllocator` structs
- Implement `new()` and `add_region()` (calculates chunk count, assigns bitmap offset, trims unaligned tails)
- Register module in `kernel/src/lib.rs`
- Write tests: add one region, add multiple regions, region with non-aligned size gets trimmed, exceeding max regions

### Step 2: Implement `allocate()`
- Scan bitmap to find `n` contiguous free bits within a single region
- Mark found bits as used (set to 1)
- Return base pointer calculated from region base + chunk offset
- Write tests: allocate single chunk, allocate multiple contiguous chunks, allocate until full returns `None`, allocate from second region when first is full

### Step 3: Implement `deallocate()`
- Find which region the pointer belongs to
- Calculate chunk index from pointer offset
- Clear the corresponding bits in the bitmap
- Write tests: allocate then deallocate, deallocate and re-allocate same space, deallocate creates gap that allows new contiguous allocation

### Step 4: Implement `free_chunks()` / `used_chunks()` + final edge case tests
- Count set/unset bits across all regions
- Write tests: counts after various allocate/deallocate sequences, empty allocator, full allocator
