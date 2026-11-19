# x86_32 context switch and the initial task frame

## Why this is hard
`swap_context` saves nothing but callee-saved general registers and the stack pointer, and a new task must look, on its first `ret`, exactly like a suspended one. The initial frame (`seed_task_frame` in `arch/x86_32/src/cpu.rs`) and the assembly (`swap_context` in the same file) are two halves of one contract.

## Invariants relied on
- Arguments are read from `[esp+4]` (store pointer, may be null) and `[esp+8]` (load value) before any push, so offsets stay fixed.
- When `store` is null the current context is abandoned: nothing is saved.
- A suspended task's stack holds, from lowest address: `edi`, `esi`, `ebx`, `ebp`, then the return address (the task entry point, popped by `ret`).
- The initial stack pointer is aligned to 16 and then lowered by 8, so after `ret` the task sees `esp = 12 (mod 16)` as the System V i386 ABI requires at function entry.
- cdecl arguments sit above the entry point: `param1` at `[esp+4]` after `ret`, `param2` above it. Above those sits a zero sentinel return address, so a task function that returns crashes visibly instead of executing garbage.
- The four initial callee-saved values are zero; the last one written is the new task's initial `esp`.
- `swap_context` starts with `cli` and never re-enables: resumed code runs with IF clear. No floating-point or other state is saved, which is the 486 baseline rule (no FP on any 32-bit target).

## What breaks if changed
Reordering pushes against pops, or changing the frame size in only one half, resumes tasks with wrong registers or a misaligned stack. Adding saved state on one side only corrupts every switch. Re-enabling interrupts inside the routine reintroduces the park-registration race described in `x86_32-int80-entry.md`.

## Evidence
Host and QEMU suites on x86_32 (`ROSX_ARCH=x86_32 cargo test -p tests-integration`), the task-stack watermark checks in `kernel/src/task_stack.rs`, and `plans/x86_32-wake-race.md`.

## Status
verified in QEMU; unverified on hardware (checklist in `docs/architecture/real-hardware-x86_32.md` once moved).
