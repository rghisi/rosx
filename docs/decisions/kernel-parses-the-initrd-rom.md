---
status: decided
date: 2026-11-22
revisit-when: a second arch or a multiboot-era loader needs the ROM under a different contract, or the initrd grows to carry more than the ROM
---

## Question

Who validates the initrd, and how much does the kernel understand of it? `plans/rom-as-initrd.md` §2.5 left this open: stage2 could hand the kernel an opaque byte range, or the kernel could parse the `[RxROM1]` image. A related open question was the 32-bit width check on the initrd slice.

## Chosen

The kernel parses the ROM. The chain contract stays generic — stage2 loads any regular file `INITRD.BIN` into low memory and publishes `BootInfo.initrd_addr/initrd_size`; nothing about the ROM format leaks into stage2. The kernel is the single authority on its own invariants: `platform::bootinfo::initrd()` maps and bounds the slice (refusing a range outside usable/conventional memory, overlapping the kernel image span, exceeding the kernel load high-water, or exceeding `INITRD_MAX_SIZE`), and `kernel::rom::validate()` checks the `[RxROM1]` directory before anything is scheduled. Each refusal panics naming its case. The init program is looked up by the name `init` (`kernel::rom::program()`).

The width check uses the compiled target's `usize::MAX`: on x86_64 every address fits, so a 64-bit kernel trusts the 32-bit wire address; on x86_32 the wire is the native width. A future arch whose pointer width exceeds the BootInfo wire width needs a real narrowing check first — recorded in `docs/rationale/x86-bootinfo-consumer.md`.

## Alternatives

- Stage2 parses the ROM and exposes per-file addresses: rejected — the bootloader would own a format the kernel consumes, splitting the invariant across two binaries and widening the frozen boot ABI for no gain.
- The kernel treats the initrd as opaque until `init` decodes it: rejected — a corrupt image would surface as a late, user-side failure while the kernel already booted services on garbage; validation belongs before boot services are scheduled.
- A generic width flag in BootInfo: rejected for now — there is no such consumer, and the plan's R4 analysis is recorded instead.

## Consequences

- Boot failures for a bad ROM are named at the kernel edge (`no initrd`, `initrd refused`, `initrd is not a ROM image`, `initrd carries no init`), and the boot-test and integration gates anchor on those texts.
- `INITRD_MAX_SIZE` (16 MiB) is now an enforced kernel bound; a disk with a giant `INITRD.BIN` fails at boot with a named panic instead of corrupting memory.
- The crc32 handshake the bootloader plan sketched (09-initrd-crc) is dead: containment and format validation are the load-time guards, and a checksummed-but-bogus ROM would still have been refused downstream.
