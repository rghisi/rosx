# Investigation Ideas

Backlog of architectural investigations to evaluate when there is time. Neither is committed work; both were discussed during the kernel-owned-handle run (PR #58, 2026-10-11) and deferred.

## 1. trybuild-lite: run the handle type-shape proof inside `cargo test`

Replace the manual `checks/compile_fail.sh` gate with a zero-dependency host integration test (e.g. `usrlib/tests/handle_kind_guards.rs`).

**Why.** `checks/compile_fail.sh` (added in `ca5bd4f`, plan step 18, requirement R13) proves the three `usrlib` handle kinds are compile-time distinct (`UserConnectionHandle` ↛ `UserFutureHandle`, no binding into `ipc_send_value`, no fabrication from a raw word). Verified facts from PR #58: `usrlib` builds and tests on the host, and the probe properties are pure library-surface type properties — the bare-metal pipeline is not required to check them. Today the script mutates a tracked file (`apps/shell/src/shell.rs` + trap/restore) and is **run by nobody automatically**: CI (`cargo xtask test --skip-integration`) never invokes it, so it can silently rot between manual runs.

**Sketch.** One host integration test that, per probe snippet, materializes a scratch crate under `target/` (outside the workspace, own lockfile — the mechanism trybuild itself uses), spawns `cargo check`, and asserts exit != 0 with `error[E0308]` in stderr (keep the substring semantics; format-drift-immune on nightly). Plus one positive-control snippet that must compile. Runs inside `cargo test -p usrlib` → `cargo xtask test` → existing CI Unit Tests job. Then retire `checks/compile_fail.sh` and note the guard in AGENTS.md test section.

**Alternative considered.** Real `trybuild` — rejected for this repo: ~20 third-party dev-deps and `.stderr` files that pin rustc rendering (re-bless churn on the pinned nightly toolchain).

## 2. rust_arkitect: architecture tests for the crate/module graph

Evaluate [`rust_arkitect`](https://crates.io/crates/rust_arkitect) (ArchUnit/phparkitect-style rule DSL over parsed `use` relationships) as an automated guard for the structural invariants currently written as prose in AGENTS.md/README.md and checked by hand-run greps.

**What it can and cannot express (conclusion from the PR #58 discussion).**

- Can express: R11 dependency rules (`usrlib -> system` only, `kernel -> system` only, …), R2-style "system names no domain types", layer rules, cycles. Parse-level, so re-export laundering can slip through; never typechecks, so it cannot replace idea 1 — the two are orthogonal oracles.
- Cannot express: R13 token-carrier discipline (no layout-constant duplication, no bit arithmetic in usrlib), type opacity — those stay with compile-fail probes + greps + review.

**Caveats found during evaluation.** `rust_arkitect` is 0.3.x and young, but has the workspace-aware API (`Project::from_current_workspace`, `rules_for_crate`) that fits this workspace shape. The sibling `arch_test_core` / `cargo-archtest` looks like the wrong fit: 0.1.5 published 2026-09-27, AGPL-3.0, pulls the rust-analyzer syntax front-end (`ra_ap_syntax`), and its `ModuleTree::new("src/lib.rs")` reads single-project-oriented.

**Zero-dependency middle path (possibly sufficient).** A `cargo xtask check-arch` subcommand: `cargo metadata` allow-list of deps per crate (catches `system` ever gaining a dependency) + the R2/R5 absence greps. The RosX crate graph is ~5 edges and Cargo itself already refuses illegal arrows; the ArchUnit DSL earns its dev-dep only if module-level accretion *inside* `kernel/` becomes a real decay mode — that is the trigger condition to revisit this.

## Related open gap (decided, not yet done)

Neither idea is wired in today, so `checks/compile_fail.sh` remains a manual gate; at minimum it deserves a CI invocation and an AGENTS.md mention until idea 1 lands.

## Resolved keyboard: shifted characters are not typeable (decided 2026-10-20, resolved 2026-10-25)

Resolved by the INPUTD migration: the stateless in-kernel decoder is gone, and the PS/2 decoding moved to the user-space driver `apps/ps2d`, whose `Ps2Decoder` is a pure state machine keeping left/right shift press and release state (`0x2A`/`0x36`/`0xAA`/`0xB6`) with a shifted US overlay (`-` → `_`, digits → symbols, letters → uppercase, releases silent), covered by host unit tests including the `0x2A 0x0C 0xAA` → `_` sequence. CapsLock remains out of scope.

**Follow-up, still open.** The ROM app names were renamed underscore-free on 2026-10-20 to mask the old gap; with shifted characters now typeable those renames can be reverted, as a separate future task.

## Deferred from the user-space allocator plan (decided 2026-11-01, from `plans/userspace-allocator.md` M4)

The binned-slab allocator ships as an opt-in (`slab-allocator` usrlib feature; the default global allocator stays the syscall passthrough). Item 0 gates flipping it on; the three follow-ups stay deliberately out of scope and none blocks the others.

**0. Task-group memory ownership in the kernel — prerequisite for making `slab-allocator` the default.** The allocator caches chunks in per-image statics (plan §2.7), but user blocks are stamped `BlockOwner::Task` and `release_task_allocations` fires when *any* task exits (plan §2.5). A task exiting from `Syscall::spawn` therefore hands its slabs back to the kernel while chunks it freed are still threaded on the image's shared free list — the next pop dangles and corrupts the kernel heap (reproduced during M3 staging: `bench alloc_churn` went totally console-silent after the first spawned bench suite terminated; plan §2.14). Hence the feature gate and the `#[ignore]`d `tests-integration/tests/bench_allocator.rs`. **Sketch.** The unit of sharing in the flat address space is the loaded image, so make it the unit of ownership: a fresh ELF load creates a group, `Syscall::spawn` siblings join the caller's group, user blocks get `BlockOwner::Group(id)`, and group memory is reclaimed only when the group's last live member terminates. Handle `pack()` — `(generation << 16) | index` with a 15-bit generation — is recycle-safe, so a group table keyed by that token works if a stable id is needed. **Verification.** Un-ignore `bench_allocator.rs` (its BAD-FREE transcript guard is the soundness oracle), flip `usrlib/src/runtime.rs` from `SyscallAllocator` to `UsrAllocator`, and run the bench/stress/threads suites plus a new `bench_baseline` record.

**1. Task-local caches (jemalloc-style tcache) behind a `TaskId` syscall.** The plan guards all bins with one global spinlock (`plans/userspace-allocator.md` §3.4) precisely because there is no way for a task to name itself: no TLS, no current-task-id syscall. A minimal `TaskId = 38` syscall returning `current_task().pack()` would let each task keep a private LIFO cache in front of the shared bins — the exact structure jemalloc's tcache gives the arena — turning the common alloc/free into an unsynchronized pop/push on task-private memory and confining the spinlock to refill/overflow. **Sketch.** Task takes/returns chunks through its own cache; on miss it batch-fetches from (and on over-full batch-returns to) the shared class bin under the lock. In the flat address space a plain `static` is shared, not task-private (plan §2.7), so the cache lives in a small global slot table: index by low bits of the `TaskId`, store the owning `TaskId` in the slot header, and a task finding its slot owned by another id falls back to the shared bin — correctness never depends on the cache. The `TaskId` value doubles as the `BlockOwner::Task` token the kernel already stamps (plan §2.3). **Caveat.** A new syscall number is an ABI change to `system` and both arch ports' dispatch; schedule it with, not inside, the M1–M3 work.

**2. Slab trim — return fully-free slabs to the kernel.** v1 never frees slabs back (plan §1 Non-Goals); exit-time ownership reclaim is the net. That means a task whose high-water mark was large keeps those slabs for its whole life. **Sketch.** Track, per class, a slab whose chunks are all in the free bin; on such a detection, `Dealloc` that whole slab back to the kernel (it carries `BlockOwner::Task`, so this is symmetric with `alloc_with_owner`). **Dependency.** Best composed with the dlmalloc-style arena (`plans/userspace-allocator.md` §5): a coalescer absorbs trimmed slabs, merges adjacent free memory, and re-hands it out, instead of trim only being able to `Dealloc` whole slabs.

**3. `realloc` fast path.** Today the runtime inherits `GlobalAlloc`'s default `realloc` (alloc-new + copy + dealloc-old), so growing a `Vec` across a chunk boundary always moves it. **Sketch.** When a chunk is the most-recently-carved tail of its slab and the next region is untouched, extend it in place for a same-or-smaller class; shrink within the same class is already a no-op because class membership is a pure function of the layout. Keep it optional: the win is only for tail growth, and the invariant (must return a pointer valid for the *new* layout) has to hold under the same routing-symmetry rule as §3.2.

## Deferred hardening: ELF load failure must not panic the kernel (decided 2026-10-19)

`elf_task_wrapper` (`kernel/src/task.rs`) calls `load_elf(...).unwrap()` when the task first runs. That is safe today because every schedulable ELF is baked in at build time (`include_bytes!` boot roster and shell statics). Once the shell dispatches unknown commands as files (`rose> snake fast` resolves ROM/VFS bytes at runtime), any name becomes schedulable: `rose> notes.txt` or a corrupted blob turns a typo at the prompt into a kernel panic. Deferred out of the shell-launches-from-ROM work by explicit decision; must land before any untrusted-bytes source (RAM files are already writable).

**Sketch.** `elf_task_wrapper` matches on the `load_elf` result instead of unwrapping: on `Err`, print a diagnostic (task name and `ElfError` debug) through the kernel output and terminate the task without entering the entry point; only `Ok(image)` proceeds. Proven by kernel build plus a QEMU run scheduling a non-ELF blob; host-testable pieces of the decision logic get unit tests per house style.
