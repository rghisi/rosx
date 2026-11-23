---
status: open
date: 2026-11-23
revisit-when: user space runs outside ring 0 or gets its own page tables
---

## Question
`RomMap` returns a raw address (`RomRef.start`) that user space dereferences directly. Is
that sound, and when must it change?

## Chosen
Yes today, on both x86 arches: one flat address space, a single page table with an
identity window (x86_64 additionally aliases it at the higher half), no user/supervisor
page split, and no SMEP/SMAP/CR0.WP enforcement. The kernel hands out the alias it already
uses for the ROM slice and `init` reads through it. Accepted as a standing assumption, not
a verified hardware property: S0/S3 evidence comes from QEMU on both arches.

## Alternatives
Copy pages into a user allocation per map (defeats the zero-copy goal); stage a real
user-visible mapping in the page tables (needs per-task tables that do not exist); keep
`RomRead` chunked copies (the path this plan removes).

## Consequences
If user space ever runs at ring 3 with its own tables, or SMEP/SMAP/CR0.WP is enabled,
`RomMap` must map the image into the reader's address space instead of aliasing kernel
memory; the `RomRef` wire format itself survives that change. `docs/rationale/rom-map-release.md`
holds the mechanism detail.
