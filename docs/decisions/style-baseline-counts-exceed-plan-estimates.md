---
status: decided
date: 2026-11-19
revisit-when: baseline cleanup rounds begin
---

## Question
The plan estimated 186 non-doc `//` lines plus 54 doc-comment lines and a single `#[allow]`; the first baseline run found more. What is frozen?

## Chosen
The baseline freezes what the tree has today: 719 offending comment lines across all tracked `.rs` files (non-doc `//`, block comments and every doc comment, test modules and tooling included), and `#[allow(` in seven files (`arch/x86-generic/src/vga_buffer.rs`, `fs/ramfs/src/volume.rs`, `fs/rxfs/src/volume.rs`, `kernel/src/handle.rs` x5, `kernel/src/ipc/ipc_manager.rs`, `kernel/src/kernel_cell.rs` x2, `kernel/src/memory/bitmap_chunk_allocator.rs`). Per-crate `unsafe` counts match the plan (kernel 289). The 32-bit hazard rule found zero hits, so it has no baseline and no `cfg(target_pointer_width = "64")` exemption yet.

## Alternatives
Trim the baseline to the plan's numbers by cleaning code first: out of scope for the gate rollout.

## Consequences
Cleanup of the baselined comments and `#[allow]` attributes is a later round; the ratchet only moves down.
