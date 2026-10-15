# Implementation Plan — TaskStack extraction

> **Date:** 2026-10-15 · **Baseline HEAD:** `2ebf6e7` ("Fix debug-only triple fault by building Task in place")
> **Inputs (binding):** `plans/taskstack-extraction.md` (brief; §8 of the context map overrides it) ·
> `.opencode/artifacts/taskstack-context-map.md` (verified tree facts) ·
> `.opencode/artifacts/taskstack-baseline.md` (verifier transcript; BINDING working invocations and fingerprints).
> **User-locked decisions:** D1 (owned `Box<[usize; STACK_WORDS]>`, safe-first construction), D2 (new module `kernel/src/task_stack.rs`, alias retired, `STACK_WORDS` migrates), D3 (final gate block embedded verbatim as Step 4).
> All gate commands below are the verifier's green/red-verbatim invocations from the baseline transcript; none is `unverified`.

---

## 0. Binding resolutions (the coder follows these verbatim)

**BR-1 — Name worlds.** During the change there are two `TaskStack` names in two worlds: the private alias `type TaskStack = [usize; STACK_WORDS];` (task.rs:12, task.rs world) and the new `pub struct TaskStack` (task_stack.rs world). They coexist legally (different module scopes; verified: `TaskStack|STACK_WORDS` occurs only in `kernel/src/task.rs` across all workspace code). Every intermediate step must leave BOTH worlds compiling: Step 1 creates the new world without touching task.rs; Step 2 retires the alias and switches the task.rs world to the newtype in one edit (field, constructor, imports, test — interdependent, cannot be split without breaking the build).

**BR-2 — Construction strategy decision point.** Strategy (a) — safe `vec![…].into_boxed_slice().try_into()` — is attempted first (Step 1). It is adopted only if it compiles (adding `#![feature(boxed_slice_try_into)]` to `kernel/src/lib.rs` if and only if rustc reports E0658 for the `Box<[T]> → Box<[T; N]>` conversion) **and** its codegen is proven clean on the fresh `--debug` artifact in Step 3 (no frame ≥ 0x178, no `rep movs`, no `memcpy` call in `TaskStack::new`). If either proof fails, rewrite **only** `TaskStack::new` per strategy (b) (contract in §2.1), re-run Step 1's gate, and re-run Step 3. `task.rs` is insulated from this choice by the newtype: its text is identical under both strategies.

**BR-3 — `new_without_default` suppression.** Clippy's `new_without_default` (5 pre-existing instances in the B1 fingerprint) would fire on a public zero-arg `new()` returning `Self`. `TaskStack::new` must carry `#[allow(clippy::new_without_default)]` so the kernel-lib clippy total stays exactly 88. This attribute is a deliberate lint suppression — **flag it for review**.

**BR-4 — test-only buffer accessor.** `TaskStack::words()` must be `#[cfg(test)] pub(crate)` — a plain `pub(crate)` accessor would be dead code in the bare-metal build and add a `dead_code` warning, violating Δ-new = 0. `base()` and `size_bytes()` stay plain `pub(crate)` because `top()` keeps them live in every build.

**BR-5 — fmt trap.** NEVER run plain `cargo fmt` (it would rewrite 339 hunks across 62 files — a massive unrelated diff and a fingerprint baseline violation). `task_stack.rs` must be written in fmt-clean form by hand; its cleanness is proven by a grep over `cargo fmt --check` output (Step 1 gate). All lines added to `task.rs`/`lib.rs` must also be written fmt-clean, so the file's existing baseline hunks (4, at lines 1/6/62/249) shrink or move but never gain a new offender.

**BR-6 — SP semantics are preserved, not re-derived.** The `stack_pointer` field is seeded once from `TaskStack::top()` (one-past-top of the heap buffer — identical derivation class to today's `stack_ptr.addr() + size_of::<TaskStack>()`, task.rs:75) and afterwards diverges from `top()` exactly as today once `prepare_entry` stores the aligned, frame-below-top SP. **Do not** "simplify" `prepare_entry` to read `stack.top()`; it must keep reading `self.stack_pointer()`.

**BR-7 — vacuous pattern ban.** The brief's original big-frame regex `sub \$0x…` (single space) is vacuous against objdump's padded mnemonics (GATE D1 defect in the baseline transcript). Only the corrected POSIX form `sub[[:space:]]+\$0x[0-9a-f]{4,},%rsp` may be used, and the honest baseline for it is **6 pre-existing matches**, not zero.

---

## 1. Target state contracts

### 1.1 New file `kernel/src/task_stack.rs` (module contents, by contract)

Imports: `use alloc::boxed::Box;` and (strategy (a) only) `use alloc::vec;`. Nothing else. No comments (AGENTS.md) except the single `// Safety:` line permitted under strategy (b).

```
pub const STACK_WORDS: usize = 2048;

pub struct TaskStack {
    buffer: Box<[usize; STACK_WORDS]>,
}

impl TaskStack {
    #[allow(clippy::new_without_default)]            // BR-3
    pub fn new() -> TaskStack                        // zero-filled 16 384-byte heap buffer
    pub fn top(&self) -> usize                       // == base() + size_bytes(); one-past-top address
    pub(crate) fn base(&self) -> usize               // self.buffer.as_ptr().addr()
    pub(crate) fn size_bytes(&self) -> usize         // STACK_WORDS * core::mem::size_of::<usize>()
    #[cfg(test)]                                     // BR-4
    pub(crate) fn words(&self) -> &[usize]           // &self.buffer[..]
}
```

- `top()` is expressed as `self.base() + self.size_bytes()` — this keeps `base`/`size_bytes` live outside test builds (BR-4) and makes the self-invariant test tautology-proof only in the sense that both sides flow from the same buffer, which is the point of pinning.
- Use fully-qualified `core::mem::size_of` to avoid any import question; no `MaybeUninit` import is nameable unless strategy (b) needs it spelled in the type of the local (it may use `core::mem::MaybeUninit` fully qualified — zero `use` lines added in either strategy beyond `alloc::vec` under (a)).
- No `Drop` impl (Box frees the buffer; drop order inside `Box<Task>` gives brief-constraint-#1 atomicity for free).
- No `Default` impl (BR-3 replaces it).

**`TaskStack::new` strategy (a) — reference skeleton (contract wins over skeleton):**

```
let buffer: Box<[usize; STACK_WORDS]> = vec![0usize; STACK_WORDS]
    .into_boxed_slice()
    .try_into()
    .unwrap_or_else(|_| unreachable!());
TaskStack { buffer }
```

Prose: the `vec!` of zero-valued `usize` lowers to `alloc_zeroed` (core default impl; `MemoryManager` implements only `alloc`/`dealloc`, memory_manager.rs:71/:94 — context map §8 confirms no `alloc_zeroed` override), so the zeroing happens inside the allocation, never as a caller temporary; `into_boxed_slice` on an exact-capacity Vec is a no-op; the feature-gated `TryFrom<Box<[T]>> for Box<[T; N]>` is a length-checked pointer cast — no bulk copy. The `Err` branch is statically impossible (Vec length equals `STACK_WORDS` by construction); `unwrap_or_else(|_| unreachable!())` aborts (panic=abort) if reached.
**Forbidden forms** (each re-imports the 0x180d8-frame bug one level down — brief hard constraint #2): `Box::new([0usize; STACK_WORDS])`; any `let b = [0usize; STACK_WORDS]`; and `vec![0usize; STACK_WORDS].try_into()` **without** the intermediate `.into_boxed_slice()` (the stable `TryFrom<Vec<T>> for [T; N]` unboxes the buffer into a 16 KiB stack temporary).

**`TaskStack::new` strategy (b) — fallback contract** (use only when BR-2's proof fails; exactly one small `unsafe`, no new imports): bind a `Box<core::mem::MaybeUninit<[usize; STACK_WORDS]>>` from `Box::new_uninit()`; inside one `unsafe` block, cast its `as_mut_ptr()` to `*mut u8` and `write_bytes(0, STACK_WORDS * core::mem::size_of::<usize>())`, then `buffer.assume_init()` (same block); return `TaskStack { buffer }`. The block carries a `// Safety:` comment in the style of the existing task.rs:72 precedent, stating: the allocation is zero-filled in place before `assume_init`; `[usize; STACK_WORDS]` is `Copy` with no Drop invariants so the all-zero bit pattern is a valid initialized value; the buffer never lives as a stack temporary. **Flag this comment for review** (repo runs the documented-unsafe-exception protocol; the AGENTS.md "no comments" exception is run-recorded, not permanent). Under (b), remove `use alloc::vec;` (unused → dead-import warning → gate-7 violation) and do **not** add the lib.rs feature attribute.

**`kernel/src/lib.rs` edits (Step 1):** add `pub mod task_stack;` between the existing `pub mod task;` (line 28) and `pub(crate) mod task_manager;` (line 29). Conditionally (BR-2, only on E0658) add `#![feature(boxed_slice_try_into)]` alongside the existing crate attributes at lines 1–2 — ungated (not `cfg_attr(not(test), …)`), because the code path must compile under `cfg(test)` too.

### 1.2 `Task` after the change — same 8 fields, one type swapped

Declaration keeps fields in the current order; only the `stack` field's type changes from the retired alias to the newtype (`stack: TaskStack`, now `crate::task_stack::TaskStack`):
`name: &'static str` · `state: TaskState` · `yield_reason: Option<YieldReason>` · `stack_pointer: usize` · `entry_point: usize` · `entry_param: usize` · `stack: TaskStack` · `completion_future: Option<FutureHandle>`.
`Task` becomes ~64 B of bookkeeping owning an 8-byte pointer; the 16 KiB buffer lives in its own heap allocation. `Task` gains drop glue (dropping the owned `TaskStack`); this is safe through `GenerationalArena` — verified: `arena.rs` has no `T` bounds, no `ptr::read`/`copy_nonoverlapping`/`transmute`/`ManuallyDrop` anywhere, and its own tests already store dropful types (`String`, `ComplexType`).

**`Task::new` — signature kept verbatim** (including its unused `'a` — removing it would alter a baseline clippy site; not our business):

```
pub fn new<'a>(
    name: &'static str,
    entry_point: usize,
    entry_param: usize,
) -> SharedTask {
    let stack = TaskStack::new();
    let stack_pointer = stack.top();
    Box::new(Task {
        name,
        state: Created,
        yield_reason: None,
        stack_pointer,
        entry_point,
        entry_param,
        stack,
        completion_future: None,
    })
}
```

Prescribed exactly: the local-binding pattern (`let stack = …; let stack_pointer = stack.top();`) so the literal seeds SP from the stack; shorthand field syntax only (no `name: name` forms — clippy `redundant_field_names`). The temporary is a ~64 B struct literal + a `Box::new` call — no 16 KiB-class value ever exists on the calling stack. Zero `unsafe`, zero comments in the function.

### 1.3 UNTOUCHED surface (state verbatim; quote-checked against HEAD `2ebf6e7`)

The following MUST NOT change — not one signature, fallback, or call site:

- `kernel/src/task.rs`: `pub fn stack_pointer(&self) -> usize` (:90-92), `pub fn stack_pointer_mut(&mut self) -> *mut usize` (:93-95, body `&mut self.stack_pointer as *mut usize`), `pub fn set_stack_pointer(&mut self, new_stack_pointer: usize)` (:96-98), `pub fn prepare_entry(&mut self, cpu: &dyn Cpu)` (:144-153) — body continues to call `cpu.initialize_stack(self.stack_pointer(), self.entry_point(), self.entry_param(), 0)` and store the result via `set_stack_pointer`, then `set_ready()` (BR-6). Also untouched: `pub type SharedTask = Box<Task>;` (:15), `idle_entry`/`task_wrapper`/`elf_task_wrapper` and their `unsafe` blocks (:172, :188, :240-242 — unrelated to the buffer).
- `kernel/src/task_manager.rs` :69-81 — `get_task_stack_pointer` (`Ok(task) => task.stack_pointer(), Err(_) => 0`) and `get_task_stack_pointer_ref` (`Ok(task) => task.stack_pointer_mut(), Err(_) => null_mut()`).
- `kernel/src/state.rs` — `switch_to_task` (:22-40) and `switch_to_scheduler` (:43-60), including `self.preemption_enabled = true;` at :33 (context map §8.5: do not resurrect reverted D1/D2/FIX-E variants).
- `kernel/src/kernel.rs` — `Task::new("[K] Main Thread", main_thread_run as usize, 0)` (:119), the three `prepare_entry(cpu)` call sites (:130/:164/:289), boot swap `swap_context(state_pointer, null_mut(), scheduler_thread_stack_pointer)` (:177-178).
- `Cpu` trait — `kernel/src/cpu.rs:6-12` `initialize_stack(&self, stack_pointer: usize, entry_point: usize, param1: usize, param2: usize) -> usize` and `:22-27` `swap_context(&self, state: *mut usize, stack_pointer_to_store: *mut usize, stack_pointer_to_load: usize)` with its aliasing-barrier doc (:13-21).
- x86_64 impl — `arch/x86_64/src/cpu.rs:68-117` (re-align `sp = (sp_val & !0xF) as *mut usize;` at :80, 16-word/0x80 entry frame) and `swap_context` :119-130 → `context_switching.S`.
- x86_32 impl — `arch/x86_32/src/cpu.rs:40-79` (`let mut sp = ((stack_pointer & !0xF) - 8) as *mut u32;` at :50) and `swap_context` :81-91.
- Test stub — `kernel/src/memory/memory_manager.rs:128-129` fake-Cpu no-ops.
- All 5 `Task` producers keep compiling with zero edits: `FunctionTask::new` (task.rs:160-163), `idle_task_factory` (:165-168), `new_entrypoint_task` (:178-180), `new_elf_task` (:182-185), `Kernel::new` direct call (kernel.rs:119).

**Why heap alignment suffices (no `repr(align)` anywhere):** SP is the buffer's one-past-top address, 8-aligned by construction (`Box<[usize; N]>` align = `align_of::<usize>()` = 8; the free-list rejects align > 8 anyway — context map §9). Both trampolines re-align **downward** before writing (`& !0xF` x86_64 cpu.rs:79-80; `(& !0xF) - 8` x86_32 cpu.rs:50) and then write at most 0x80 B (x86_64) / 0x24+8 B (x86_32) into the buffer. 16 384 B ≫ 0x80; every written byte lies inside the allocation.

---

## 2. Steps

### Step 1 — Create `kernel/src/task_stack.rs` + declare it in `kernel/src/lib.rs` (strategy-(a) probe)

**Files:** `kernel/src/task_stack.rs` (new), `kernel/src/lib.rs` (one line, plus conditional feature attribute).

1. Write `task_stack.rs` per §1.1 using strategy (a). Include its `#[cfg(test)] mod tests` now:
   - `top_is_one_past_the_top`: `TaskStack::new()`; `assert_eq!(stack.top(), stack.base() + stack.size_bytes());`
   - `buffer_is_zero_filled`: `assert!(stack.words().iter().all(|w| *w == 0));` (same closure form as the existing test at task.rs:221 — no new clippy surface).
   - `addresses_are_nonzero`: `assert_ne!(stack.base(), 0); assert_ne!(stack.top(), 0);`
   Do **not** assert the literal 16384 (host-size-independent formulation via `size_bytes()` only).
2. `kernel/src/lib.rs`: insert `pub mod task_stack;` after `pub mod task;` (line 28).
3. `task.rs` remains byte-identical in this step (BR-1: the private alias at :12 keeps the old world compiling).

**Strategy probe (part of this step):** if the test compile fails with E0658 on the `try_into()`, add `#![feature(boxed_slice_try_into)]` to lib.rs per §1.1 and re-run. If the conversion does not exist under any feature spelling on the pinned nightly (rustc 1.101.0-nightly 0abfedbc7), switch `TaskStack::new` to the strategy-(b) contract (§1.1), remove `use alloc::vec;`, and re-run. The strategy decision is finalized in Step 3's disassembly proof; this step decides only compile-availability.

**Gate 1 (all three):**
- `cargo test -p kernel -- --test-threads=1` → exit 0, all pass. Baseline claim: **221 passed / 0 failed** (verifier transcript GATE A, exact). Expect 224 after +3 new tests; Δ-new failures = 0; record the actual total.
- Compile-warning posture (from GATE A's own capture, "lib: 60 warnings, lib test: 86 warnings"): zero NEW warning texts attributable to `task_stack.rs`.
- New-file fmt cleanness: run `cargo fmt --check > /tmp/taskstack-fmt.log 2>&1` then `grep -E 'task_stack\.rs' /tmp/taskstack-fmt.log` → **zero matching lines** (the exit code of `cargo fmt --check` is expected 1 at baseline; do not judge by it; BR-5: never run plain `cargo fmt`).

### Step 2 — Rewire `kernel/src/task.rs` (one atomic edit set)

**File:** `kernel/src/task.rs` only. Sub-edits (interdependent; execute as one commit; BR-1 forbids splitting):

1. **Imports (lines 6-7, deleted in full):**
   - `use core::mem::{size_of, MaybeUninit};` — `size_of` appears only in the deleted region (:75, :82; the test at :219 uses fully-qualified `core::mem::size_of_val`), `MaybeUninit` only at :70.
   - `use core::ptr::{addr_of_mut, write, write_bytes};` — all three used only inside the deleted `Task::new` block. **Verified safe re: the Display impl's `write!` macros (:36-48):** that is the `write!` macro (macro namespace), not `core::ptr::write` (value namespace); the same crate already uses bare `write!(f, …)` with no import at `keyboard.rs:347` (whose only fmt import is `use alloc::fmt::{Display, Formatter};`), so the macro resolves from the core prelude on the pinned nightly. Contingency only if the bare-metal build disagrees: add `use core::write;`.
   - Add `use crate::task_stack::TaskStack;` among the `use crate::…` lines (:1-2).
2. **Retire the alias (D2):** delete `const STACK_WORDS: usize = 2048;` (:11) and `type TaskStack = [usize; STACK_WORDS];` (:12). `STACK_WORDS` now lives in `task_stack.rs` (domain knowledge owned by the allocating subsystem, AGENTS.md).
3. **Replace `Task::new` body (:65-86)** with the plain-safe struct-literal form of §1.2. The `// Safety:` comment at :72 and the whole `MaybeUninit`/field-wise-`write` block disappear (its run-recorded comment exception retires with it — context map §9).
4. **Migrate the SP-pins test** `task_new_field_integrity` (:207-222), keeping every existing assertion, now via the newtype:
   - `assert_ne!(task.stack_pointer(), 0);` (:216) — unchanged.
   - `assert_eq!(task.stack_pointer(), task.stack.as_ptr().addr() + core::mem::size_of_val(&task.stack),);` (:217-220) → `assert_eq!(task.stack_pointer(), task.stack.top());`
   - `assert!(task.stack.iter().all(|w| *w == 0));` (:221) → `assert!(task.stack.words().iter().all(|w| *w == 0));` (`task.stack` is private but visible to the test module in the same file; `words()` is the BR-4 `#[cfg(test)]` accessor).
   - Leave `new_task_has_no_completion_future` (:224-228) and `can_set_and_get_completion_future` (:230-236) untouched.
5. All other in-file code (Display impl, producers, wrappers, `make_future_handle`) byte-identical.

**Gate 2 (all four; each is a verifier-proven invocation):**
- `cargo test -p kernel -- --test-threads=1` → exit 0; Δ-new failures = 0 vs the 221-baseline set (the other `Task::new` test sites from context map §4 — `task_manager.rs:111/:119`, `scheduler/mod.rs:92/:106`, `future.rs:204/:220`, `kernel_services.rs:141`, `ipc/mailbox_manager.rs:191` — compile untouched and keep passing).
- `cargo xtask build` → exit 0 (the bare-metal `no_std` compile — this is the gate that catches any no_std-only surprise, e.g. the `write!` contingency above).
- From `arch/x86_32/`: `cargo build -p rosx-x86` → exit 0 (shared-kernel parity; its `.cargo/config.toml` supplies the target and `-Z` flags — run from that directory, no extra flags).
- Warning posture: no NEW compile-warning texts; removals from the deleted `Task::new` region are expected decreases (attribute by multiset diff, never as absolute counts).

### Step 3 — Strategy confirmation on the fresh debug artifact (cheap static probe, BEFORE any QEMU)

**Files:** none (verification only; loop-back target is `task_stack.rs::new` per BR-2).

1. `cargo xtask build --debug` → exit 0, fresh `target/rosx/debug/rosx` (baseline artifact was 8,887,312 B; mtime must be post-edit — cargo freshness suffices).
2. Big-frame scan on the dump: `objdump -d target/rosx/debug/rosx | grep -E 'sub[[:space:]]+\$0x[0-9a-f]{4,},%rsp'` → exactly the recorded 6-line baseline multiset {0x1018 ×2, 0x1028, 0x1068, 0x40b8, 0x10d8}, none in a task/task_stack symbol. (BR-7. A 7th frame in task/task_stack = hard fail; a new frame elsewhere = Δ-new violation, escalate, do not shrug.)
3. Frame attribution on the dump (raw v0-mangled symbol names; the `_ZN` awk filter from the old delegation matches nothing — transcript D2 note):
   - `Task::new`: locate with `grep -nE '^[0-9a-f]+ <.*4Task3new.*>:'` (baseline symbol `_RNvMs_NtCsfT1KQWZ1OH8_6kernel4taskNtB4_4Task3new` @ 0x288050; the address will drift — size is the fingerprint).
   - `TaskStack::new`: locate with `grep -nE '^[0-9a-f]+ <.*9TaskStack3new.*>:'` (disjoint from the `4Task3new` pattern — `9TaskStack` carries the length prefix `9`).
   - In each symbol's block, read the first `sub[[:space:]]+\$0x[0-9a-f]+,%rsp`. **Both must be < 0x178**; record both verbatim.
   - Copy check inside `TaskStack::new`'s block only: `grep -cE 'rep[[:space:]]+movs'` == 0 and no `memcpy` call (BR-2 "no heap-to-heap bulk copy"). Note: `rep stos`/fill evidence elsewhere (e.g. `__rust_alloc_zeroed` under strategy (a), or inside `TaskStack::new` itself under strategy (b)) is the intended zeroing-in-place, not a copy.
4. **Loop-back (BR-2):** any failure above ⇒ strategy (a) is rejected ⇒ rewrite `TaskStack::new` per the (b) contract, re-run Gate 1 and Gate 2, re-run Step 3. `task.rs` is not re-touched.

### Step 4 — Final acceptance gates (VERBATIM block; run in this exact order)

Ordering law: **gate 4 runs on the fresh gate-2 `--debug` artifact, before any QEMU run** — the integration harness rebuilds `target/rosx/debug/rosx` mid-suite with its own dev profile (transcript E1), so post-QEMU fingerprints are unattributable. One QEMU at a time, always `-- --test-threads=1`.

1. `cargo test -p kernel -- --test-threads=1` → all pass; baseline 221 (transcript GATE A, exact); Δ-new failures = 0; total may grow with the new `TaskStack` tests — record actual.
2. `cargo xtask build` → exit 0; `cargo xtask build --debug` → exit 0 producing fresh `target/rosx/debug/rosx`.
3. `cargo build -p rosx-x86` FROM `arch/x86_32/` → exit 0.
4. Big-frame scan (RUN ON THE FRESH --DEBUG ARTIFACT, BEFORE any QEMU run): `objdump -d target/rosx/debug/rosx | grep -E 'sub[[:space:]]+\$0x[0-9a-f]{4,},%rsp'` → must match the recorded baseline frame-size multiset {0x1018 ×2, 0x1028, 0x1068, 0x40b8, 0x10d8} exactly (6 lines, none attributed to task/task_stack code — addresses drift freely, sizes and attribution are the fingerprint; baseline attribution: 4 IDT-bootstrapping + 2 shell String-sort monomorphs, transcript D1). Δ-new big frames = 0. The single-space regex is the vacuous pattern and must never be used as evidence.
5. Frame attribution: extract the `sub $0x..,%rsp` prologue for `Task::new` (baseline 0x178, transcript D2) and for `TaskStack::new` — BOTH must be < 0x178 and recorded verbatim. Working discovery commands per Step 3.3.
6. QEMU tier, one QEMU at a time, `--test-threads=1`:
   - `cargo test -p tests-integration --test app_lifecycle -- --test-threads=1` → exit 0, 2/2 (`tetris_survives_idle`, `snake_survives_idle` — now plain `#[test]`, no `#[ignore]` since 2ebf6e7; expect `running 2 tests`, not vacuous).
   - `cargo test -p tests-integration --test boot_and_shell -- --test-threads=1` → exit 0, 4/4 (`running 4 tests`).
   - `cargo test -p tests-integration --test ipc_random -- --test-threads=1` → exit 0, 1/1 (`running 1 test`).
   - Note (transcript E1/E2a/E2b): build output interleaves into the test stream; `ok` tokens may appear detached — judge by the `test result: ok. N passed; 0 failed` summary lines.
7. Lint/format Δ-new gates against the transcript's re-captured expected-red fingerprints (exit codes are baseline-red; the fingerprint match is the gate — do NOT "fix" the red):
   - `cargo clippy --workspace --all-targets` (capture exit + full log; never pipe the exit through `tail`) → exit 101 with exactly the recorded B1 fingerprint: 2 rustc errors (`#[alloc_error_handler]` conflict; `error[E0152]: found duplicate lang item panic_impl` → `arch/x86_64/src/main.rs:55:1`) + 1 cargo summary; kernel (lib) 88 warnings; 125 `-->` diagnostic-location lines. **Δ-new = 0**: zero new lint instances at new locations; the stale `handle.rs:268`/`absurd_extreme_comparisons` claim is NOT the baseline (0 matches on the pinned nightly — transcript B1 divergence note). Decreases attributed by multiset diff (lint-name × file) to deleted `task.rs` sites are expected, never violations; the new file `task_stack.rs` must contribute **zero** warning instances (BR-3 is why).
   - `cargo fmt --check` (capture exit + log) → exit 1 with the recorded B2 fingerprint: 339 hunks / 62 files, task.rs = 4 hunks at lines 1, 6, 62, 249. Attribution rule: **deleted-site decreases are expected** — task.rs hunks at 6 and 62 may vanish or shift because lines 6-7/11-12 are deleted and :62-86 is rewritten; a hunk-count ≤ 4 with only shifted/moved markers is compliant. **NEW failing files are forbidden**: no file outside the 62-file inventory may appear; `task_stack.rs` must be entirely fmt-clean (absent from the log — already proven at Gate 1, re-prove here).

---

## 3. Blast radius: every process kind that executes the changed constructor

The change replaces what the `Task` constructor wires behind indirect plumbing (5 producers, global service registry, both arch targets). Test processes in radius, with the process kind each proves:

| Process kind | Proof |
|---|---|
| Host unit (std, cfg(test)) | `cargo test -p kernel -- --test-threads=1` — covers task.rs tests (3, one migrated), task_stack.rs tests (+3), and the 11 `Task::new` test sites in `task_manager.rs:111/:119`, `scheduler/mod.rs:92/:106`, `future.rs:204/:220`, `kernel_services.rs:141` (shared static `KernelServices` — single-thread mandatory), `ipc/mailbox_manager.rs:191` — these transitively reach `Task::new` through the global service registry, not only directly. |
| x86_64 bare-metal ELF | `cargo xtask build` (release) + `cargo xtask build --debug` (gate-4 artifact; the debug profile is the one that historically materialized the 0x180d8 frame). |
| x86_32 parity ELF | `cargo build -p rosx-x86` from `arch/x86_32/` (shared `kernel/` code). |
| Live boot + survivors (runtime SP witness) | QEMU tier gate 6: boot swap (`kernel.rs:177-178`), scheduler/idle `prepare_entry` paths, snake/tetris surviving idle — the only processes where a wrong SP actually faults. |
| Disassembly (codegen witness) | Gates 4/5 on the fresh debug artifact. |

## 4. Risks and edge cases (coder must handle)

1. **Feature unavailability (strategy (a)).** `boxed_slice_try_into` may be absent/renamed/stabilized on the pinned nightly; handle per BR-2 in order: try → E0658 ⇒ add feature attr ⇒ if still unresolvable ⇒ strategy (b) contract. Never improvise a third construction (no `MaybeUninit<Task>`, no array literal).
2. **Big-value reintroduction.** Forbidden spellings in §1.1; the Step 3 probe is the mechanical backstop (frame < 0x178, no `rep movs`). `vec![…]` alone is not safe — the `.try_into()` on a `Vec` (missing `.into_boxed_slice()`) unboxes into a 16 KiB stack temporary.
3. **Lint-quietness of new code.** BR-3 (`new_without_default`), BR-4 (`words()` dead-code), shorthand field syntax (`redundant_field_names`), no unused imports in either strategy branch (`alloc::vec` under (b) only if used).
4. **Gate-5 boundary.** If `Task::new` or `TaskStack::new` measures ≥ 0x178, treat as a stop: inspect for unintended temporaries first; if the excess is benign debug spill with gate 4 still exactly the 6-frame baseline, halt and escalate to the orchestrator — do not silently relax a binding gate.
5. **`unreachable!()` under panic=abort.** Provably unreachable (Vec length ≡ `STACK_WORDS`); if reached the kernel aborts — acceptable and strictly better than UB; no special handling.
6. **SP numeric values change, semantics don't.** SP now addresses a separate heap allocation, not the interior of the Task box. Pinned by the migrated invariant test (`stack_pointer == stack.top()` at construction) and by the QEMU tier (post-`prepare_entry` behavior). `prepare_entry` keeps reading `stack_pointer()` (BR-6).
7. **Heap adjacency.** A stack overflow now corrupts a neighboring heap chunk instead of Task bookkeeping. Not a gate; a behavior note for reviewers (overflow is equally fatal either way today).
8. **Allocation growth accounting.** +1 heap allocation per task (≤256 tasks, arena cap task_manager.rs:8); Task box shrinks 0x4030 B → ~64 B; total per-task memory ≈ neutral (16 KiB buffer + ~64 B bookkeeping). Free-list (align ≤ 8) accepts both allocations.
9. **fmt discipline.** BR-5: plain `cargo fmt` is forbidden for the whole task; only the grep-probe over `--check` output is used.
10. **Expected-red gates.** Clippy and fmt are red at baseline (2 lang-item host-target errors; 339 fmt hunks). A coder must not "fix" them (workspace host-build is intentionally unsupported — AGENTS.md); Δ-new rules in gate 7 govern.
11. **Artifact ordering.** Gates 4/5 must read the pre-QEMU `target/rosx/debug/rosx` (Step 4 ordering law). If any QEMU run happens first, re-run `cargo xtask build --debug` and redo 4/5 before judging.
12. **Drop-glue ordering.** `Box<Task>` drop → `Task` fields drop → `TaskStack` (owning `Box`) frees the buffer, all inside one arena `remove` (`task_manager.rs:31-33` `let _ = tasks.remove(...)`); no window exists in which the stack outlives the task or vice versa (brief constraint #1). No Drop impl to write — don't add one.

## 5. Acceptance criteria (restated)

All of Step 4, gates 1–7, in order, with: unit Δ-new failures = 0 against the 221 baseline (actual total recorded); both builds exit 0 with fresh debug artifact; x86_32 parity exit 0; big-frame multiset exactly {0x1018 ×2, 0x1028, 0x1068, 0x40b8, 0x10d8} with Δ-new = 0 and no task/task_stack attribution; `Task::new` and `TaskStack::new` prologues both < 0x178 and recorded verbatim; QEMU tier 2/2 + 4/4 + 1/1 exit 0; clippy exit 101 at the exact B1 fingerprint and fmt exit 1 at the exact B2 fingerprint with Δ-new = 0 under the stated attribution rules, `task_stack.rs` absent from the fmt log.

## 6. Revert story

The change is confined to three files: `kernel/src/task.rs` (modified), `kernel/src/task_stack.rs` (new), `kernel/src/lib.rs` (one `pub mod` line, plus conditional one-line feature attribute). Reverting the commit restores the PR #60 state (`2ebf6e7`: in-place `MaybeUninit` construction, inline buffer) exactly — no migration, no state on disk, no other file touched.
