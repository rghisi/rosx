---
status: open
date: 2026-11-22
revisit-when: a caller must distinguish an ELF rejection from other LoadElf failures
---

## Question

How does a `LoadElf` caller learn that its ELF was rejected?

## Chosen

The existing generic failure: `LoadElf` returns `u64::MAX as usize`, which userspace already
decodes as `None` (`usrlib`'s `Syscall::load_with_args`), and the kernel prints one diagnostic
line (`[ELF] '<name>' rejected: <ElfError>`) through kernel output. No new `system::error::ErrorCode`
variant: the `Message` layout and the error-code table stay untouched (no ABI change).

## Alternatives

A dedicated `ElfLoadRejected` `ErrorCode` (an ABI change in `system`, needs approval): only worth
it when a caller must branch on the failure kind rather than report it.

## Consequences

Callers see a failed launch, not its cause; the cause is on the kernel console. Adding the code
later is additive and keeps this record's revisit trigger honest.
