# ELF Loading at the Syscall — Implementation Log

Evidence log for `plans/elf-load-at-syscall.md`. One section per milestone, appended as each lands.
Branch: `elf-load-at-syscall`, started at `83ade61` (identical to `origin/main` at start).

## M0 — Characterisation (no production change)

Gate for M1: the plan's `[U]` items are decided here. **None contradicts a plan premise, so M1
proceeds on the plan as written.**

### U1 — the loaded image is not group-owned: PROVEN

Chain: (a) `load_elf` builds the image with `vec![0u8; max_addr]` (`kernel/src/elf/mod.rs:83,138`),
so the block comes from the `#[global_allocator]`, which is `MEMORY_MANAGER` in the kernel build
(`kernel/src/memory/memory_manager.rs:10-11`); (b) that path is
`GlobalAlloc::alloc -> alloc_with_owner(layout, BlockOwner::Kernel)` (`memory_manager.rs:178-181`)
— never `Group`; (c) new unit test
`memory::memory_manager::tests::global_allocator_blocks_are_invisible_to_group_accounting_and_sweep`
allocates a 196 KiB align-1 block (the `vfsd`-sized image shape) through that exact function and
asserts `group_bytes(g) == 0` for every `g` and that `release_group_allocations` — the same sweep
both death paths run (`kernel.rs:332,385` via `free_list_allocator.rs:283-301`, which matches
`Group` only) — leaves it allocated.

Harness caveat recorded honestly: under `cargo test` a `Vec` is backed by the host allocator, so no
host test can push `load_elf` itself through `MEMORY_MANAGER`; the test therefore drives the
production `GlobalAlloc` implementation directly, and links (a)+(b) by code reading.
`group_bytes` counts `Group(g)` + `Stack(g)` only (`free_list_allocator.rs:268-281`), so the kernel-
stamped image is excluded from per-group accounting — U1 holds. Consequence confirmed: today's
`kill_reclaims_the_group_only_when_the_last_member_dies` test (`kernel.rs`) never exercised image
reclaim because nothing image-shaped was ever group-stamped.

### U2 — `Task`'s Drop runs after `handle_termination` -> `remove_task` on both death paths: PROVEN

`Drop`-based reclamation is reachable: `remove_task` (`task_manager.rs:50`) removes the
`SharedTask` from the arena, and dropping it runs the field drops (the `TaskStack::Drop` dealloc,
`task_stack.rs:97-101`, is the observable marker — `deallocate_by_owner` never frees `Stack`-stamped
blocks, so a vanishing `Stack` block can only come from `Task`'s Drop).

- **Kill path** — `kill_task` ends with `scheduler.handle_termination(owner)` (`kernel.rs:388`) ->
  `remove_task` (`scheduler/scheduler.rs:145-151`). New unit test
  `kernel::tests::kill_drops_the_task_whose_stack_bytes_the_sweep_never_frees`: a spawned task's
  group is non-empty (its stack), and `group_bytes(group) == 0` immediately after `kill_task` —
  with no `Group`-stamped block in play, the free must be `Task`'s field drops.
- **Exit path** — `terminate_with_status` (`kernel.rs:312-339`) marks the task `Terminated` and
  switches to the scheduler; the scheduler-side step is
  `KernelContextSwitcher::switch_to_task` reporting `SwitchOutcome::Terminated` (`kernel.rs:47-55`)
  -> `reconcile_returned_task` (`scheduler/scheduler.rs:168-183`) -> `handle_termination` ->
  `remove_task` (that mapping itself: scheduler unit test
  `run_one_round_terminates_task_returned_in_terminated_state`, `scheduler.rs:637-653`). New unit
  test `kernel::tests::exit_drops_the_task_when_the_scheduler_removes_it`: the stack block survives
  `terminate_with_status` (the `Task` is still in the arena while the last-member sweep runs — the
  exact reason stacks stay out of the sweep) and is freed exactly by the following
  `handle_termination`.

Consequence for the design: `Task`-owned (`Drop`) storage *is* reachable (U2 true), but the plan's
D3 rejection stands on a different ground (F8: `Spawn` siblings execute code inside the first
task's image, so per-`Task` drop can free the image under a live sibling). D3 (group ownership)
stands.

### U3 — alignment of today's image: ANSWERED

`load_elf` ignores `p_align` entirely; the image is a `Vec<u8>` (`Layout` align 1). In the kernel's
allocator that takes `plan_carve`'s `align <= BLOCK_ALIGN` branch (`free_list_allocator.rs:63-65`):
pointer = free-block start + `ALLOC_HDR`, every free-block start sits on the `align_of::<FreeBlock>()`
grid = `align_of::<usize>()` (8 B on x86_64, 4 B on x86_32), and `ALLOC_HDR` (32 B on x86_64, 16 B
on x86_32) is a multiple of that grid. New unit test
`memory::memory_manager::tests::align_one_allocations_land_on_the_block_alignment_grid` asserts the
8-byte floor for 64 B..196 KiB align-1 allocations (host measures the x86_64 case; the x86_32
numbers follow from the same type arithmetic).

So today's images are guaranteed only usize-aligned — and that already works, because relocation
writes are `read_unaligned`-based. `readelf -l` on the shipped release apps (vfsd, shell, init,
ne2kd, both arches) reports `p_align = 0x1000` for every PT_LOAD.

**M1 alignment contract (recorded here per the plan):** `Layout.align = clamp(max PT_LOAD p_align,
16, 4096)`. The allocator supports every power-of-two align with checked arithmetic (huge aligns
`Err(OutOfMemory)`, never corrupting: tests `huge_alignment_attempts_leave_the_free_list_untouched`
and `absurd_alignment_returns_out_of_memory`; overalignment up to 4096 exercised by
`overaligned_allocations_honor_alignment_and_free_clean`), so the cap at 4096 is a policy choice —
the largest shipped `p_align` exactly — not an allocator limit.

### Baseline `kstats_mem` used-KiB (for M4 test 9)

Measured on this machine (host QEMU + mtools present; release kernels built by
`cargo xtask build <arch>`; `ROSX_KERNEL_ELF` pointed at them; steady state = after `rose>`, five
`mem` reports each):

| Arch | used KiB (5 samples) | M0 baseline |
|---|---|---|
| x86_64 | 2341 x5 | **2341 KiB** |
| x86_32 | 2074 x5 | **2074 KiB** |

Expected after this plan (M4 test 9): drop by roughly the retained-copy figure — the plan's table
re-derived against the current release builds: the 11 steady-state programs total 601 424 B =
587.3 KiB (x86_64) and 578 488 B = 564.9 KiB (x86_32), matching the plan's 587/565 KiB. Predicted
steady state: ~1754 KiB (x86_64), ~1509 KiB (x86_32).

### M0 verdict

U1 proven, U2 proven, U3 answered; no premise contradicted; plan needs no amendment. Unit tests
added: 4 (`cargo test -p kernel`: 479 passed). No production behaviour changed. The two allocator
tests each carry one `unsafe` block (the tested functions are `unsafe fn`), raising the kernel
`unsafe` ratchet 300 -> 302; decision record
`docs/decisions/gate-baseline-unsafe-raised-2026-11-21-2.md` was written by
`cargo xtask check-style --update-baseline --allow-increase`.

## M1 — Two-phase pure loader (`elf::inspect` + `elf::load_into`)

`kernel/src/elf/mod.rs` rewritten:

- `inspect(bytes) -> Result<ElfLayout, ElfError>` validates the header, bounds-checks the phdr
  table itself (the old code read `e_phoff`-derived pointers unguarded), and for every PT_LOAD
  checks `p_filesz <= p_memsz`, `p_offset + p_filesz <= file` (checked), `p_vaddr + p_memsz`
  overflow-free, and `e_entry < span`; returns `{ span, align, entry_offset, class }` before any
  allocation (threat T3). New variants `MalformedSegment`, `SpanTooLarge`.
- Span cap: `MAX_IMAGE_SPAN = 4 MiB` — decision record `docs/decisions/elf-image-span-cap-4mib.md`.
- Alignment contract exactly as M0 recorded: `align = clamp(max power-of-two PT_LOAD p_align, 16,
  4096)`; non-power-of-two or < 2 p_align falls back to the floor. (Not consumed until M2.)
- `load_into(bytes, dst, layout, arch) -> Result<usize /*entry*/, ElfError>` copies PT_LOADs with
  slice ops and applies PT_DYNAMIC relocations **against `dst.as_ptr()`**, with every raw read and
  every relocation write target bounds-checked against `dst` (the old code could read the dynamic
  table or patch outside the image). `load_into` requires `dst.len() == layout.span` and zeroed
  (debug-asserted; slice checks make release misuse an `Err`, never UB).
- All raw reads go through one safe helper `read_at::<T>` — the crate's only new `unsafe` site;
  the old nine scattered `unsafe` blocks are gone and the elf module's total `unsafe` tokens
  stayed at 16, so the 302 ratchet is unchanged.
- `load_elf`/`Image` survive as a thin wrapper over `inspect` + `load_into` (still the production
  path from `elf_task_wrapper`); they are deleted in M3 when the wrapper's caller migrates —
  never left as a test-only item.

Test-1 note on the plan's list: "`p_vaddr + p_filesz` past `span`" is not constructible once
`p_filesz <= p_memsz` holds and `span = max(p_vaddr + p_memsz)`; the hostile ELF it describes is
rejected by the `p_memsz < p_filesz` check (`MalformedSegment`). Subsumed case, recorded here per
the plan's M0 amendment duty; both are `Err`, neither a panic.

Tests (plan test 1 and 2): 16 new unit tests — rejections (`TooSmall`, magic, class,
`NoLoadSegments` via `inspect` directly; beyond-file, `filesz > memsz`, overflowing bounds, phdr
table beyond EOF, span cap, entry at-or-past span, huge `r_offset`, rela table past span),
byte-identical `load_into` output for the ELF32 and ELF64 fixtures, relocations addressed against
the caller's `dst`, layout fields and alignment clamp. `cargo test -p kernel`: 495 passed; full
`cargo xtask check` green.

## M2+M3 — Group-owned program memory, eager constructor, load at the syscall

**Fold rationale (deviation from the plan's commit split):** the plan's M2 is a types-only step
(`ProgramMemory` + ownership skeleton, no caller). That cannot pass the gate: `unused = deny`
(`[workspace.lints.rust]`) flags a `ProgramMemory` nothing constructs, and the bridge caller would
either be test-only production surface (banned by AGENTS.md) or M3's production change itself. M2
and M3 therefore land as one reviewable commit.

- `kernel/src/memory/program_memory.rs` (new): `ProgramMemory` — one block stamped
  `BlockOwner::Group(g)` via `alloc_with_owner`, zeroed at allocation, with **no `Drop`**: the only
  way it is freed is the group's last-member sweep (`release_group_allocations` matches `Group`
  only), which is exactly why per-`Task` drop can never free the image under a live `Spawn`
  sibling (D3/F8). Safety in `docs/rationale/group-stamped-program-memory.md`, linked from the two
  doc comments on items containing `unsafe`.
- `kernel/src/task.rs`: `new_elf_task_with_arguments(name, elf, blob, &dyn ElfArch) ->
  Result<SharedTask, ElfError>` is now eager — `inspect` → `GroupId::allocate` → one
  `ProgramMemory` block laid out `[image | ElfProgram record @ align_up(span, 8) | argv table |
  argument strings]`; `load_into` writes the image in place, argv strings and the NUL-split argv
  table and the `{entry, argv, argc}` record are written into the same block. On any
  `load_into` failure the group is swept before returning `Err` — nothing leaks. The task entry is
  `elf_task_wrapper(record_addr)`, which now only `read_unaligned`s the record and jumps: the
  first slice parses and loads nothing, and no `Vec` of ELF bytes survives the syscall anywhere.
  The M1 `load_elf`/`Image` wrapper is deleted in this commit (its last caller migrated).
- `kernel/src/kernel.rs`: `load_elf_program(name, elf, blob, owner) -> Result<FutureHandle,
  ElfError>` = eager constructor + `schedule_with_task_handle`; a schedule failure sweeps the group
  and returns the new `ElfError::CannotBeScheduled`. `schedule_with_task_handle` returns
  `Err(())` instead of `panic!("Not able to create new task")`. Both death paths already called
  `release_group_allocations` at the last member (M0: `kernel.rs:332,385`); the image simply joins
  that sweep now that it is group-stamped (U1 fixed by construction).
- `kernel/src/syscall.rs`: `LoadElf` runs `load_elf_program`; rejection prints
  `[ELF] '<name>' rejected: <error>` and returns `u64::MAX` — no wire change (the old path already
  returned the packed completion handle on success and `u64::MAX` on failure), so per-error detail
  stays kernel-side by decision `docs/decisions/elf-load-failure-generic-error-code.md`.
- `arch/x86_64/src/main.rs`, `arch/x86_32/src/main.rs`: the init ELF goes through
  `load_elf_program` too; boot failure is a loud `panic!("init ELF rejected: {error:?}")`.
- `docs/todo/elf-load-failure-must-not-panic-kernel.md` deleted (git rm) — a malformed ELF fails
  `LoadElf` with a diagnostic; only the boot-path init keeps a panic, where there is no one left
  to report to.
- Tests: `kernel.rs` — `a_schedule_failure_after_elf_construction_releases_the_group` (registry
  exhausted → `CannotBeScheduled`, heap used byte-identical), and the two
  `a_loaded_program_group_survives_until_the_last_member_dies_via_kill`/`_via_exit` tests: with a
  sibling alive, the first member's death keeps the group non-empty (image + sibling stack), and
  the last death brings `group_bytes` to 0; `program_memory.rs` — group stamp + zeroing, sweep
  reclaims even written-into blocks, oversize allocation returns `None` with nothing stamped;
  `task.rs` — rewritten around the record (argv written into the record, the record's argv
  addresses the group's copy and not the caller's blob, empty and empty-string argvs, null-terminated
  three-argument table, the image is group-owned, a load failure after allocation releases the block,
  the group holds the image not a copy of the ELF file, distinct groups per load). `cargo test -p
  kernel`: 496 passed.
- `unsafe` ratchet: kernel **302 → 295** — the eager constructor and the record-reading wrapper
  remove more `unsafe` than `ProgramMemory` adds; ratcheted down with `--update-baseline` (drops
  need no decision record).

## Friction log

M0 | test import of `BLOCK_ALIGN` failed E0603 — constant is private, visibility not a plan fact | MISSING-FACT | 1 round
M0 | first `cargo xtask check` failed `unsafe: kernel has 302, baseline allows 300` — new cfg(test) code counts toward the ratchet | MISSING-FACT | 1 round + decision record
M2+M3 | plan's types-only M2 commit cannot pass the gate (unused=deny + no-test-only-production-surface) | CONTRADICTION | plan commit split amended: M2 folded into M3
M2+M3 | check-style reported "302 → 292" while `program_memory.rs` was untracked — survey scans `git ls-files`, so new files only count after `git add` | MISSING-FACT | 1 round
M2+M3 | `NonNull::addr()` returns `NonZero<usize>` on this nightly — needs `.get()` for plain `usize` | MISSING-FACT | 1 round
