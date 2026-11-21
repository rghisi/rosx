# ELF Loading at the Syscall — No Retained ELF Copy, Group-Owned Program Memory, No Kernel Panic on Bad Input

> **Status:** Plan only. Nothing in this document has been executed. Written 2026-11-21 against `54034e0`.
> **Scope:** v1 moves ELF loading from the new task's first slice (`elf_task_wrapper`) into the task-creation call
> (`LoadElf` syscall and the boot path), stops retaining a copy of the ELF bytes for the process lifetime, puts the
> loaded image and the argument vectors under the existing loaded-image-group ownership so every death path reclaims
> them, and turns a malformed ELF into a failed `LoadElf` instead of a kernel panic.
> **Origin:** the kernel memory-footprint analysis (opportunity #1, 565-587 KiB steady-state) and the follow-up
> discussion that loading directly from the caller's buffer removes the intermediate copy.
> **Closes:** `docs/todo/elf-load-failure-must-not-panic-kernel.md` (deleted in the commit that lands M3).
> **Method:** every code fact below was read in the working tree on 2026-11-21 and cites its file; items that were
> inferred rather than read carry **[U]** and are proven or refuted by M0 before any design depends on them.
> **Parent documents:** `docs/architecture/memory.md` (group ownership), `plans/kill-resource-sweep.md` (the reap this
> plan must stay consistent with), `plans/taskstack-extraction.md` (task-owned buffers freed by `Drop`).

## 0. Goal and non-goals

### Goal

1. After `load_elf` finishes, the kernel holds no copy of the ELF file. Steady state keeps only the loaded image
   (the PT_LOAD span, smaller than the file) plus argv.
2. The peak during a load drops by the ELF file size (no `to_vec()` next to the image).
3. The loaded image and argv are freed on **every** death path (clean return, `Exit` syscall, `Kill`, user panic),
   when the last member of the loaded-image group dies, never earlier.
4. A malformed or non-ELF blob makes `LoadElf` fail for the caller; the kernel does not panic.

### Non-goals

- No ABI or wire change: the `LoadElf` signature, `system::error::ErrorCode` and the `Message` layout stay as they
  are (D1). A dedicated ELF-failure error code is a recorded deferral, not part of this plan.
- No change to ELF format support (ELF32/ELF64 PT_LOAD + PT_DYNAMIC relocations), to relocation semantics, or to how
  user programs are linked.
- No page-granular memory, protection or demand loading; the image stays one contiguous heap block.
- No change to task-stack sizing (separate footprint item).
- No third-party crates; no widening of the 486 feature floor (nothing here needs a new instruction or atomic).

## 1. Verified baseline facts

Cited against `54034e0`.

| # | Fact | Where |
|---|---|---|
| F1 | `LoadElf` receives a borrowed `&[u8]` from the caller, validates the argument blob, then calls `new_elf_task_with_arguments(name, elf_bytes, Vec::from(blob))` | `kernel/src/syscall.rs:118-137` |
| F2 | `ElfTaskContext::with_blob` does `elf_bytes: elf_bytes.to_vec()`; the box's raw pointer becomes the task's `entry_param` | `kernel/src/task.rs:245-260`, `306-309` |
| F3 | The new task, on its first slice, rebuilds the box, calls `load_elf(&context.elf_bytes, kernel().elf_arch).unwrap()` and then calls the entry point; the context (bytes included) is dropped only after the entry returns | `kernel/src/task.rs:563-583` |
| F4 | `Image` is `{ image: Vec<u8>, entry: usize }`, with no lifetime parameter, so it cannot borrow the ELF bytes; `load_elf64` builds `vec![0u8; max_addr]`, copies each PT_LOAD segment and applies PT_DYNAMIC relocations against `image.as_ptr()` | `kernel/src/elf/mod.rs:28-31`, `54-106` |
| F5 | The global allocator stamps every `alloc` as `BlockOwner::Kernel`; only the `Alloc` syscall and task stacks use `alloc_with_owner` with `Group`/`Stack`. So the `Image` `Vec` and the argv `Vec`s are Kernel-owned, not group-owned | `kernel/src/memory/memory_manager.rs:178-181`, `kernel/src/syscall.rs:101-113`, `kernel/src/task_stack.rs:39` |
| F6 | `Exit` calls `terminate_with_status` directly, and `kill_task` never resumes the victim; neither path returns into `elf_task_wrapper`, so the wrapper's `drop(image)` / `drop(context)` run only when the entry point returns normally. On `Exit`, `Kill` and a user panic (`usrlib` panics call `Syscall::exit`) the image and context are not freed by the wrapper | `kernel/src/syscall.rs:70-73`, `kernel/src/kernel.rs:312-339`, `358-391`, `usrlib/src/runtime.rs:57` |
| F7 | Both death paths reclaim the group only when its last live member dies: `release_group_allocations(group)` sweeps `BlockOwner::Group` blocks, never `Kernel` blocks | `kernel/src/kernel.rs:329-333`, `382-387`, `kernel/src/memory/free_list_allocator.rs:283-301` |
| F8 | `Spawn` siblings join the caller's group and share that image's statics, so sibling tasks execute code that lives in the first task's `Image` | `docs/architecture/memory.md:11`, `kernel/src/task.rs:236-238` |
| F9 | `elf_task_wrapper`'s `.unwrap()` turns any `ElfError` on user-supplied bytes into a kernel panic (`panic = "abort"`) | `kernel/src/task.rs:565`, `docs/todo/elf-load-failure-must-not-panic-kernel.md` |
| F10 | Callers of the loading entry point: the `LoadElf` syscall, and both arch `main.rs` files for the embedded `init`. Userspace callers already treat a failed `LoadElf` as `None` (`Syscall::load_with_args` returns `Option`) | `arch/x86_64/src/main.rs:96`, `arch/x86_32/src/main.rs:75`, `usrlib/src/syscall.rs:36-51`, `apps/init/src/main.rs`, `apps/shell/src/launcher.rs:107` |
| F11 | Unit coverage exists for argv construction and for `load_elf` over minimal ELF32/ELF64 fixtures (`build_minimal_elf`); one test, `elf_task_context_owns_a_copy_of_the_image`, asserts the behaviour this plan removes | `kernel/src/task.rs:362-410`, `550-560`, `kernel/src/elf/mod.rs:374-594` |

Derived sizes (file sizes of the shipped release ELFs, measured 2026-11-21; heap totals derived by hand):

| Quantity | x86_64 | x86_32 |
|---|---:|---:|
| ELF copies held by the 11 steady-state programs (`atad blockd vfsd inputd ps2d randomd echo pong pcid ne2kd shell`) | 587 KiB | 565 KiB |
| Largest single copy (`vfsd` file / loaded span) | 196 KiB / about 144 KiB | 198 KiB / not measured |
| Intermediate copy removed from the load peak | the file size of the program being loaded | same |

**[U1]** The image is not group-owned, so `group_bytes(group)` excludes it (follows from F5; M0 proves it with a test).
**[U2]** `Task`'s `Drop` runs after `handle_termination` -> `remove_task` on both death paths (`scheduler.rs:145-151`, `task_manager.rs:50`), so a `Drop`-based design is reachable; M0 confirms.
**[U3]** Today `Image` alignment is whatever the allocator happens to return for a `u8` `Vec`; ELF `p_align` is not consulted by `load_elf`. M1 decides the alignment contract.

## 2. Decisions

| # | Decision | Default (proceed on this) | Needs your approval? |
|---|---|---|---|
| D1 | Failure signal for a bad ELF | Return the existing generic failure (`u64::MAX as usize`, which userspace already decodes as `None`). No new `ErrorCode`. Record a `docs/decisions/` deferral with `revisit-when: a caller must distinguish ELF failure from other LoadElf failures` | Only if you want a new `ErrorCode` (an ABI change in `system`) |
| D2 | Boot-path failure of the embedded `init` | The arch `main` panics with a named message (`init ELF rejected: {ElfError:?}`): the ELF is baked in at build time, so failure is a build defect and unrecoverable | No |
| D3 | Who owns the loaded image and argv | **Loaded-image group**, via `alloc_with_owner(Group)`; the existing last-member sweep frees it on `Exit`, `Kill` and clean return. See section 3.3 for why `Task`-owned (`Drop`) storage was rejected as the default | Only if the M2 unsafe-count increase needs the decision record the gate requires |
| D4 | Argv storage | Same group-owned allocation as the image (tail of the block), one allocation and one lifetime | No |
| D5 | `load_elf` / `Image` public surface | Replaced by a two-phase API (3.1); the old names are deleted, not kept as test-only wrappers (house rule: no test-only production surface) | No |

## 3. Design

### 3.1 Two-phase pure loader (`kernel/src/elf/`)

Keep `elf` free of allocation policy (architecture principle: the allocating subsystem owns the allocation).

```
elf::inspect(bytes: &[u8]) -> Result<Layout, ElfError>
    // validates header + phdrs; returns { span, align, entry_offset, class }
elf::load_into(bytes: &[u8], dst: &mut [u8], layout: &Layout, arch: &dyn ElfArch) -> Result<usize /*entry*/, ElfError>
    // dst is zeroed, len == span; copies PT_LOAD, applies PT_DYNAMIC relocations against dst.as_ptr()
```

- `inspect` runs the checks `load_elf32/64` run today (size, magic, class, `NoLoadSegments`) and also bounds-checks every
  PT_LOAD (`p_offset + p_filesz <= bytes.len()`, `p_vaddr + p_filesz <= span`), which the current indexing does not
  guard by construction (it slices and would panic) **[U]**; those become `ElfError` returns, which is part of goal 4.
- Alignment contract (U3): `Layout.align` is `max(16, max PT_LOAD p_align)` capped at the allocator's supported maximum;
  M1 reads `plan_carve` to confirm the cap and records it.
- Both functions stay host-testable with a `Vec` as `dst` and the existing `MockElfArch` and `build_minimal_elf` fixtures.

### 3.2 Eager, fallible task construction (`kernel/src/task.rs`)

```
new_elf_task_with_arguments(name, elf, blob, elf_arch: &dyn ElfArch) -> Result<SharedTask, ElfError>
    1. layout  = elf::inspect(elf)?
    2. group   = GroupId::allocate()
    3. block   = group-owned allocation of (image span + argv area), zeroed        // None -> Err(OutOfMemory)
    4. entry   = elf::load_into(elf, block.image(), &layout, elf_arch)              // Err -> release group, return Err
    5. argv    = write blob + pointer table into block.argv()
    6. task    = Task::in_group(name, elf_entry_trampoline, ptr_to_program_record, group)
```

- `Task::in_group` becomes `pub(crate)`; the group is allocated by the caller of `in_group` so memory can be stamped
  before the `Task` exists.
- Every failure after step 3 calls `release_group_allocations(group)` so a rejected or unschedulable program leaves no
  stamped block behind. **`Kernel::schedule` failing after a successful construction** (`TaskCannotBeAdded`, arena full) must
  release the same way; M3 adds that path and a test.
- `elf_task_wrapper` becomes a plain entry call: read the program record (entry, `argc`, `argv`) through `entry_param`,
  set `preemption_enabled`, call the entry, `terminate_with_status`. It owns nothing, so there are no drops left to skip on
  `Exit`/`Kill`/panic. The `unsafe` count at that site goes from two (`Box::from_raw`, `transmute`) plus the
  `Box::into_raw` in construction to one pointer read plus the `transmute`; the baseline only ratchets down.

### 3.3 Why group-owned, not `Task`-owned (D3)

The attractive alternative is a `program: Box<ElfProgram>` field on `Task`, freed by `Task::drop` on every path (like
`TaskStack`). It is rejected as the default because of F8: `Spawn` siblings execute code inside the first task's image.
If the first task's `Task` drops first (an `Exit` from the main thread while a sibling runs), a `Drop`-owned image is freed
under the sibling. The group sweep waits for the **last** member (F7), which is the lifetime the image actually has, and it
is the invariant `docs/architecture/memory.md` already states for user memory. (Today's wrapper has the same latent hazard
for a clean return with live siblings; group ownership closes it.)

Fallback if group-stamped allocation costs too much unsafe: an `Arc<ElfProgram>` cloned into each group member at `Spawn`
(`new_entrypoint_task`). That is safe Rust but threads a handle through `Spawn` and adds an atomic refcount; revisit only if
M2's decision record is declined.

### 3.4 Group-stamped program memory (`kernel/src/memory/`)

A small type owns the unsafe: it calls `MEMORY_MANAGER.alloc_with_owner(Layout, BlockOwner::Group(g))`, zeroes the block, and
hands out `&mut [u8]` for the load and a read-only view afterwards. It has no `Drop` (the group sweep frees the block; an early
free would double-free against the sweep). The existing `Alloc` syscall already stamps group blocks the same way, so this
adds one construction site, not a new mechanism. Raising the crate's `unsafe` count needs a record in `docs/decisions/`
(`check-style --update-baseline --allow-increase "<reason>"` writes it); M2 does that if the count rises.

### 3.5 Error mapping at the edges

- `LoadElf` syscall: `Err(_)` -> `u64::MAX as usize` (D1); the kernel prints one diagnostic line (task name, `ElfError` debug) through kernel output.
- Arch `main.rs`: `Err(e)` -> `panic!("init ELF rejected: {e:?}")` (D2).
- `docs/architecture/memory.md` gains the invariant: *ELF image and argv are group-owned; the kernel retains no ELF bytes after a load.*

## 4. Safety argument

- **Lifetime of code under live threads:** the image is freed only by the last-member sweep (F7), so no sibling ever
  executes freed code; the only change in who runs the sweep is that it now has something to sweep.
- **No new exposure of unvalidated bytes to the allocator:** `inspect` validates before any allocation sized by the ELF;
  `span` is capped (M1 chooses the cap from the largest shipped image with headroom, recorded in `docs/decisions/` if non-obvious).
- **Interrupt discipline:** the load now runs in the caller's syscall, which is already kernel code with IF=0 (the same
  non-preemptible window the new task's first slice has today). Latency is charged to the caller; M4 measures boot time.
- **Reap ordering:** nothing in `reap_task_resources` changes; the stamped block is freed by the existing
  `release_group_allocations` call, after portio revoke and future reclaim, exactly as user blocks are today.
- **Stack vs heap:** stacks stay `BlockOwner::Stack` and are not swept; unchanged.

## 5. Change surface

| Area | Files | Change |
|---|---|---|
| ELF format | `kernel/src/elf/mod.rs`, `structs.rs` | split `load_elf` into `inspect` + `load_into`; bounds returns instead of panics; delete `Image` and `load_elf` |
| Task creation | `kernel/src/task.rs` | fallible `new_elf_task_with_arguments`, program record, simplified `elf_task_wrapper`, delete `ElfTaskContext::elf_bytes` |
| Memory | `kernel/src/memory/` | group-owned program-memory type |
| Syscall | `kernel/src/syscall.rs` | map `Err` to the existing generic failure; diagnostic line |
| Boot | `arch/x86_64/src/main.rs`, `arch/x86_32/src/main.rs` | pass the `ElfArch`, handle `Err` (D2) |
| Docs | `docs/architecture/memory.md`, delete `docs/todo/elf-load-failure-must-not-panic-kernel.md`, `docs/decisions/` records for D1 and any unsafe raise | |
| Tests | `kernel` unit tests; `tests-integration` (section 6) | |

Untouched by design: `system` (ABI), `protocol/*`, `usrlib`, every app, `Cargo.toml` dependencies, `xtask/arch-rules.txt`.

## 6. Test plan

Unit (host, `cargo test -p kernel -- --test-threads=1`), written before the code they cover:

1. `inspect`: rejects `TooSmall`, `InvalidMagic`, `UnsupportedClass`, `NoLoadSegments` as today; new cases: PT_LOAD extending past the file, `p_vaddr + p_filesz` past `span`, `p_memsz < p_filesz`, overflowing offsets: each an `Err`, none a panic.
2. `load_into`: byte-identical image to today's `load_elf` for the 32-bit and 64-bit fixtures; relocations applied against the destination's address; entry offset.
3. Program memory: stamped as `Group(g)`; `group_bytes(g)` includes it (proves/refutes U1's mirror image); `release_group_allocations(g)` returns it to the free list.
4. Construction: success yields a task whose group owns the image; each failure after allocation releases the group (`group_bytes == 0`); a `schedule` failure releases it.
5. Death paths: following `kill_reclaims_the_group_only_when_the_last_member_dies` (`kernel/src/kernel.rs`), a loaded-program group survives the first member's death and is fully reclaimed when the last dies, via both `Exit` and `Kill`.
6. Replace `elf_task_context_owns_a_copy_of_the_image` with an assertion that no ELF bytes are reachable from the program record.

Integration (QEMU, both arches; `cargo xtask test`):

7. A non-ELF blob started from the shell (the `rose> notes.txt` case in the todo) reports a failed launch and the system stays up.
8. Spawn-then-kill cycles: `kstats_mem` used-KiB is stable across N cycles of load, run and `Kill` (extends `kstats_mem.rs` / `ctrl_c.rs` patterns).
9. Steady-state used-KiB after boot is lower than the M0 baseline by roughly the section 1 figure; recorded in the impl log with both numbers.
10. `measure_boot` shows no boot-time regression beyond noise.

## 7. Milestones (each = one reviewable commit, `cargo xtask check` green)

| M | Content | Done when |
|---|---|---|
| M0 | **Characterise, no production change.** Tests/measurements proving U1 and U2, a baseline `kstats_mem` used-KiB per arch, and the actual alignment of today's image (U3). Creates `plans/elf-load-at-syscall-impl.md` (evidence log) | the three unknowns are answered in the log; any contradicting a premise amends this plan before M1 |
| M1 | `elf::inspect` + `load_into` with tests 1-2; delete `load_elf`/`Image` once callers migrate (or keep them only until M3 inside this commit's tests, never as a test-only item at the end) | tests 1-2 green; gate green |
| M2 | Group-owned program-memory type, test 3; `docs/decisions/` record if `unsafe` rises | test 3 green; gate green |
| M3 | Fallible eager construction, simplified wrapper, syscall and boot edges, tests 4-6; delete the todo file; D1 deferral record | tests 4-6 green; `cargo xtask check` green |
| M4 | Integration tests 7-10 on both arches; record numbers in the impl log | `cargo xtask test` green on x86_64 and x86_32 |
| M5 | `docs/architecture/memory.md` invariant; rules-ledger entry if the retro step yields one | gate green |

Each step ends with the `retro` skill's step-end mode, reported with the step's commit; a lesson that changes rules or docs
is a proposal that lands only after approval, in its own commit. Final merge evidence: `cargo xtask verify`.

## 8. Threat register

| # | Threat | Mitigation |
|---|---|---|
| T1 | Image freed under a live sibling | group ownership (3.3); test 5 |
| T2 | Stamped block leaked when construction or `schedule` fails after allocation | explicit release on every error path; test 4 |
| T3 | Hostile ELF drives an unbounded or overflowing allocation | `inspect` validates and caps `span` before allocating; test 1 |
| T4 | Load moves latency into the caller's syscall | same IF=0 window as today; test 10 |
| T5 | Alignment assumption broken for a PIE that needs more than the allocator's default | explicit `Layout.align` contract (U3); M0 measures today's behaviour first |
| T6 | A premise (U1-U3) is wrong | M0 gates M1; plan amended before code |
| T7 | `unsafe` baseline raised without a record | M2 uses `--allow-increase "<reason>"` or the `Arc` fallback |

## 9. Verification (when executed)

`cargo xtask check` on every step's head; `cargo test -p kernel -- --test-threads=1`; `cargo xtask test` on both arches for M4;
`cargo xtask verify` before merge. The authoring environment of this plan lacked QEMU and mtools, so M4 and the final
evidence need a machine or CI run that has them; the impl log must say which.

## 10. Follow-ups (trigger-gated, not in this plan)

- A dedicated ELF-failure `ErrorCode`: `revisit-when` a caller must tell ELF rejection from other `LoadElf` failures (ABI change; needs approval).
- Right-size task stacks by class (separate footprint item; independent of this plan).
- Move the embedded ROM out of the kernel image (separate footprint item).
