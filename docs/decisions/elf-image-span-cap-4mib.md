---
status: decided
date: 2026-11-21
revisit-when: a shipped program's PT_LOAD span exceeds 2 MiB, or the kernel heap grows enough that the cap stops protecting it
---

## Question

What maximum image span does `elf::inspect` accept before allocating, so a hostile ELF cannot drive an unbounded allocation (elf-load-at-syscall M1, threat T3)?

## Chosen

`MAX_IMAGE_SPAN = 4 MiB` (4 * 1024 * 1024 bytes), checked in `inspect` before any allocation; a larger span is `ElfError::SpanTooLarge`.

Largest shipped span: the `vfsd` image at about 144 KiB (about 196 KiB file); every other steady-state program is smaller. The cap is therefore about 28x the largest real image — generous for growth, tiny next to the tens of MiB kernel heap it protects, and no legitimate program loads a contiguous 4 MiB image under the current one-block-per-image design.

## Alternatives

No cap (the old `vec![0u8; max_addr]` accepted any header-computed span); a page-table demand loader (out of scope: no page-granular memory in this plan).

## Consequences

A crafted ELF is refused with `SpanTooLarge` before a byte is allocated; a future program above 2 MiB of loaded span trips the revisit trigger.
