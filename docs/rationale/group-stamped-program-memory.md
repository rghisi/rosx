# Group-stamped program memory

`ProgramMemory` (`kernel/src/memory/program_memory.rs`) is the one allocation that holds a loaded
ELF image plus its argv area. Its ownership rules:

- **Stamped `BlockOwner::Group`.** The block is freed by the loaded-image group's last-member
  sweep (`release_group_allocations`), the same reclaim that already frees user `Alloc` blocks.
  This is why the type owns the image's lifetime: `Spawn` siblings execute code inside it, so only
  the death of the group's last live member may free it (see `docs/architecture/memory.md`).
- **No `Drop` on purpose.** A `Drop` would free the block while siblings can still be executing
  from it, and would double-free against the sweep (the sweep removes the block from the allocator
  on the last member's death). Dropping the descriptor is a no-op; the raw block outlives it and
  is reclaimed exactly once by the sweep.
- **Aliasing.** The block's memory is owned by the allocator, not the type, so the `bytes`/
  `bytes_mut` views borrow the descriptor, not the storage. Callers hold at most one `&mut` view
  at a time (Rust-enforced through the descriptor). The entry trampoline copies the program record
  out through a raw pointer once, before the first instruction of user code runs; nothing in the
  kernel holds a borrow into the block while the image executes.
- **Zeroing at construction** covers the BSS tail (the span beyond `p_filesz` up to `p_memsz`), so
  the loader only copies file bytes.
