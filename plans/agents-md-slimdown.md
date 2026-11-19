# AGENTS.md Slim-Down, Mechanical Gates and Hands-Off Workflow — Plan

> **Status:** Plan only. Nothing in this document has been executed. Written 2026-11-19.
> **Goal:** shrink `AGENTS.md` from ~56 KB to ~10–12 KB without losing precision, replace prose rules with
> tool-enforced gates behind one `cargo xtask check` entry point, and make the agent workflow hands-off:
> the agent decides inside the written invariants and records deferrable decisions instead of asking.
> **Method:** every repo fact below was verified against the working tree on 2026-11-19.

**Decisions (owner-approved 2026-11-19):**
- Long architecture prose moves to `docs/architecture/<subsystem>.md`; `AGENTS.md` keeps one-line invariants and an index.
- "No comments" is enforced with a **baseline**: the existing offenders are frozen in a file, new ones fail the gate, the baseline only shrinks.
- The agent commits and pushes to its working branch autonomously when the gate is green. PR creation stays a manual (user-requested) act.
- The audit findings are recorded as the first entries of `docs/decisions.md`.

---

## 1. Findings (why)

1. `AGENTS.md` is 56 KB / 313 lines; eight paragraphs (disk, network, IP, VFS, boot plan, exit status, memory/future ownership, `fs/*` engines) are 2–5 KB each. They are design records that duplicate the code, `plans/*.md` and tests, and they go stale while the file claims to be "always-accurate".
2. Duplication: the boot plan appears in four places, the FAT boot disk in three, x86_32 integration notes in two; the directory tree duplicates `Cargo.toml`; the `xtask` list duplicates `cargo xtask help`; CI recipes are cited as canonical and then re-explained.
3. Historical narrative ("formerly `LIST_DIR`", "the inversion this rework removed", "footgun is gone") has no instructional value.
4. Rules stated in prose but not enforced, with evidence of drift:
   - "No comments, ever": 111 non-doc `//` lines exist (e.g. `kernel/src/kernel_services.rs`).
   - "`#[expect]` only, never `#[allow]`": `arch/x86-generic/src/vga_buffer.rs:18` has `#[allow(dead_code)]`.
   - `checks/compile_fail.sh` is run by nobody (`docs/todo.md` item 1).
   - Dependency arrows (`kernel → system` only, `system` has no deps) are checked by hand-run greps (`docs/todo.md` item 2).
   - The `ROSX_KERNEL_ELF` footgun: unset, the harness silently builds a dev-profile kernel (`tests-integration/src/kernel_build.rs`) and runs look like regressions.
5. The workflow contradicts hands-off operation: `AGENTS.md` ("Work Incrementally") requires approval before every change; `GEMINI.md` says "always check with me before making changes"; `.opencode/` carries its own agent definitions.

## 2. Target end state

- `AGENTS.md` ≤ ~12 KB: orientation, build/test commands, the `xtask check` contract, the invariant index, the autonomy rule, the decision-log rule.
- `docs/architecture/*.md`: the moved subsystem prose, trimmed of history, one file per subsystem.
- `cargo xtask check`: the single gate, identical locally, in the agent stop-hook and in CI.
- `docs/decisions.md`: the append-only decision/deferral backlog (merges `docs/todo.md`).
- `CLAUDE.md`, `GEMINI.md` and `.opencode` instructions all defer to `AGENTS.md`; no divergent workflow rules.

## 3. Execution order

Gates first, so the shortened prose is backed by enforcement before any rule text is deleted. Each step is one commit with no message prefix; the gate must be green before the commit (from step 1 onward).

### Step 0 — Decision log bootstrap
- Create `docs/decisions.md` with the entry format below and seed it with: the four approvals above, the findings in §1 as context, and the items from `docs/todo.md`.
- Keep `docs/todo.md` as a stub pointing at `decisions.md` until step 9 removes it.
- Entry format: `## D<n> — <title>` then `Date`, `Status` (open | decided | superseded), `Question`, `Chosen`, `Alternatives`, `Revisit when`.
- **Done when:** the file exists, entries D1–D4 recorded, todo items migrated with unchanged substance.

### Step 1 — `cargo xtask check` skeleton
- Add `check` to `xtask/src/main.rs` (`usage()` text included). It runs, in order, failing fast with a named stage: `check-style`, `check-arch`, `check-isa` (x86_32), warnings-as-errors build of host surfaces, `run_tests(skip_integration = true)`.
- Stages not yet implemented are absent, not stubbed (no dead code); each later step adds its stage.
- **Done when:** `cargo xtask check` runs the currently existing stages and exits non-zero on failure; CI's Unit Tests job calls it.

### Step 2 — `check-arch` (dependency arrows)
- New `xtask/src/check_arch.rs`: parse `cargo metadata --format-version 1`, compare each workspace crate's path-dependencies and third-party dependencies to an allow-list kept in `xtask/arch-rules.toml` (or a const table):
  - `system` has no dependencies; `kernel` depends only on `system`; `usrlib` only on `system`; `fs/*` engines only on `fs-core` (and their documented exceptions); protocol crates per their current manifests.
- First run derives the allow-list from the current manifests, reviewed by hand once, so the gate starts green and only catches future change.
- Unit-test the comparison logic with fixture metadata JSON.
- **Done when:** adding a dependency to `system` or `kernel` makes `check-arch` fail; this closes `docs/todo.md` item 2's "middle path".

### Step 3 — `check-style` (comments, `#[allow]`, unsafe, 32-bit hazards)
- New `xtask/src/check_style.rs`, scanning tracked `*.rs` (via `git ls-files`) with a small tokenizer that skips string/char literals so `//` inside strings is not counted:
  1. **Comments:** non-doc line comments (`//` not `///` or `//!`) and block comments. Baseline in `xtask/style-baseline.txt` as `path:count`. A file's count may only decrease; a new offending file or an increase fails.
  2. **`#[allow(`:** banned; the single existing occurrence is baselined; `#[expect(..., reason = ...)]` stays the sanctioned form.
  3. **`unsafe` count:** per-crate count baselined (kernel has ~289 occurrences today); an increase fails and requires an explicit baseline bump in the same commit (a recorded decision, see §5).
  4. **32-bit hazards:** `f32`/`f64` tokens and `AtomicU64`/`AtomicI64` in `kernel`, `usrlib`, `apps`, `arch/x86_32`, `arch/x86-generic`, except under `cfg(target_pointer_width = "64")`-gated items; baseline any current hits after reviewing each.
- `*.S` / `global_asm!` bodies are exempt from the comment rule (assembly must be commented).
- Provide `cargo xtask check-style --update-baseline`, which refuses to raise any count unless `--allow-increase "<reason>"` is given; the reason is appended to `docs/decisions.md` automatically.
- Unit-test the tokenizer and baseline comparison.
- **Done when:** a new `// foo` in a `.rs` file fails the gate; the baseline reflects today's tree exactly.

### Step 4 — Extend `check-isa` coverage of the 486 rules
- Keep ISA mnemonic audit as is. Fold the f32/f64/AtomicU64 check into step 3 (source level) and verify no `cmpxchg8b`/SSE mnemonic is already denied by the allow-list audit (it is an allow-list, so it is).
- Confirm the pinned feature strings (`-mmx,-sse,-sse2,-cmov,-fxsr,-cx8`) in `rosx-i486*.json` are asserted by a `check-arch` rule (string match on the target JSON) so widening them fails the gate.
- **Done when:** editing the feature strings in either 486 target spec fails `xtask check`.

### Step 5 — Compile-fail guard inside `cargo test` (docs/todo item 1)
- Implement the "trybuild-lite" host integration test in `usrlib/tests/handle_kind_guards.rs` as designed in `docs/todo.md` item 1: scratch crate under `target/`, `cargo check`, assert failure with `error[E0308]` plus one positive-control snippet.
- Retire `checks/compile_fail.sh`; remove the `checks/` directory.
- **Done when:** `cargo xtask test --skip-integration` exercises the probes and fails if a handle kind becomes interchangeable.

### Step 6 — Integration-test footgun
- `xtask test` (and `xtask check` when it runs integration) builds the release kernel(s) and sets `ROSX_KERNEL_ELF` itself.
- In `tests-integration/src/kernel_build.rs`, a missing `ROSX_KERNEL_ELF` in the default (non-explicit-dev) path fails with a message instead of silently building a dev kernel; an explicit `ROSX_QEMU_PROFILE=dev` remains the opt-in.
- **Done when:** running the integration suite without the variable either works through `xtask` with the release kernel or fails loudly, never silently slow.

### Step 7 — Move architecture prose out of `AGENTS.md`
- Create `docs/architecture/` with one file per subsystem, text moved verbatim then trimmed of history in the same commit pair (move commit first so the diff is reviewable, trim commit second):
  `scheduling-and-interrupts.md`, `ipc-and-futures.md`, `exit-status-and-ownership.md`, `memory.md`, `boot-plan.md`, `disk-and-blockd.md`, `network-and-ip.md`, `vfs.md`, `fs-engines.md`, `kstatsd-and-rom.md`, `input.md`, `real-hardware-x86_32.md` (the seven-item checklist; cross-link `arch/x86-86box/README.md`).
- Each file begins with the invariants that would silently break if violated, followed by rationale.
- Do not edit meaning; verify moved facts against code while trimming, and record any discrepancy found as a `docs/decisions.md` entry rather than silently fixing prose or code.
- **Done when:** every paragraph removed from `AGENTS.md` exists in exactly one architecture file.

### Step 8 — Rewrite `AGENTS.md`
Keep, in this order:
1. Purpose and platform status (≈10 lines).
2. Commands: `cargo xtask check` (the gate), `build`, `run`, `test`, integration invocation, pointer to `cargo xtask help` and the CI workflows. No duplicated recipes.
3. Invariant index: one line per invariant, each linking its `docs/architecture/*.md` file. Examples: kernel is platform-agnostic and `no_std`, depends only on `system`; HAL traits own hardware access; level-triggered IRQs stay masked until `IrqClear`; task stacks are not freed by group reclaim; the kernel never interprets IPC payloads; validation happens once, kernel-side; `system` holds contracts, not mechanisms; the 486 baseline.
4. Code-style rules that remain judgement-based: minimal and documented `unsafe` surface, no test-only production API, pluggable subsystems, preserve the "who owns what" principles (kept as the five short bullets).
5. Rules that are now gated are one line each: "enforced by `cargo xtask check`" (comments, `#[allow]`, unsafe baseline, dependency arrows, 486 hazards, compile-fail guards).
6. Autonomy and decision-log rules (§4).
7. Toolchain notes: nightly pin, `RUSTFLAGS` must not be exported for user ELFs, the `gh pr edit` REST workaround.

Remove: the directory tree, the boot-plan listing, the xtask command list, duplicated build recipes, historical narrative, and the "Work Incrementally / ask approval" section.
- **Done when:** size ≤ ~12 KB, and a mechanical check confirms every moved paragraph is reachable from the index.

### Step 9 — Single instruction source and cleanup
- `GEMINI.md`: replace the "always check with me before making changes" lines with `@AGENTS.md` only.
- `.opencode/` agent and command prompts: review each for ask-before-change rules that contradict §4 and either remove them or point at `AGENTS.md`. Record anything that cannot be reconciled in `docs/decisions.md`.
- Delete the `docs/todo.md` stub; update README links if any point to it.
- **Done when:** a grep for "check with me", "ask for feedback", "wait for user feedback" over instruction files returns nothing.

### Step 10 — Agent-side gate via hook
- Using the `update-config` skill, add a `Stop` hook (and optionally a pre-commit hook) in `.claude/settings.json` that runs `cargo xtask check` and blocks completion on failure with the failing stage in the output.
- Add a `.githooks/pre-commit` calling the same command and have `xtask` offer `cargo xtask install-hooks` (sets `core.hooksPath`), so non-Claude agents are covered.
- **Done when:** an intentional violation (a new `// x` comment) is rejected both by the hook and by CI.

## 4. Autonomy and decision rules (text to land in `AGENTS.md`)

- The agent proceeds without asking whenever a choice is covered by a written invariant, a gate, or an existing pattern in the repo.
- The agent commits per small, reviewable step when `cargo xtask check` is green, with no message prefix, and pushes to its working branch. It does not create PRs unless asked.
- The agent stops and asks only for: breaking an invariant or the architecture principles, new third-party dependencies (kernel allows none), ABI/wire changes in `system` or `protocol/*`, widening the 486 feature floor, claims requiring hardware evidence, deleting user data or history, and anything outward-facing beyond its branch.
- Any decision that is not needed now is appended to `docs/decisions.md` with status `open` and a `Revisit when` trigger, then work continues on the chosen default.
- Raising a gate baseline (comments, `unsafe`, allow-lists) requires a `docs/decisions.md` entry in the same commit.
- Gate failures are fixed at the cause; never skip, loosen or quarantine a gate to get green.

## 5. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Tokenizer false positives/negatives in `check-style` | Unit tests with fixtures (raw strings, nested block comments, `///` vs `////`); start with baseline equal to current counts so noise is bounded |
| Moved prose loses precision | Move-then-trim in separate commits; verify facts against code; discrepancies become decision entries |
| Agent over-autonomy on ABI-affecting changes | Explicit stop-list in §4; `check-arch` guards dependency edges |
| Hooks slow every turn | Hook runs `--skip-integration`; integration stays in CI |
| Baseline becomes a permanent excuse | `--update-baseline` refuses increases without a logged reason; revisit the baseline in a later decision round |

## 6. Out of scope

- Cleaning up the existing 111 comments, the `unsafe` surface, or the lone `#[allow]` (the baseline freezes them; cleanup is a later round).
- Adopting `rust_arkitect` (revisit only if module-level decay inside `kernel/` appears, per `docs/todo.md` item 2).
- Changing kernel behaviour, wire formats or boot plans.

## 7. Acceptance for the whole plan

1. `cargo xtask check` is green on the tree and red on each of: new `//` comment, new `#[allow]`, new dependency on `system`, widened 486 feature string, interchangeable handle kinds.
2. `AGENTS.md` ≤ ~12 KB, no content lost (every removed paragraph lives in `docs/architecture/`).
3. No instruction file asks the agent to wait for approval; `docs/decisions.md` exists with the findings and migrated backlog.
4. CI runs the same `cargo xtask check` the agent hook runs.
