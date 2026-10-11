# Kernel-Owned Handle Domain Model — Planning Spec

> **Date:** 2026-10-11
> **Branch:** `kernel-owned-handle` (from `main` @ `7171686`, post PR #57 merge)
> **Status:** DRAFT — two decisions open (§6); everything else locked in design dialogue 2026-10-11.
> **Supersedes:** decisions D2 and D4 of `plans/fixed-size-handle.md` (placement of `ERROR_BIT` and codec ownership). The wire layout D1 and everything on it stay frozen.

---

## 1. Why (motivation)

The current dependency graph is inverted. `collections` is meant to be boring, reusable infrastructure, yet it encodes three OS-level policy facts:

- `ERROR_BIT` — the **syscall register ABI**,
- the 15/16-bit generation/index split — an **ABA-safety policy** for kernel objects,
- `is_handle` — a **wire-decoding predicate**.

Meanwhile 100% of the arena's consumers are kernel modules (`ipc_manager`, `mailbox_manager`, `task`, `future`, `task_manager`). `Handle` lives in `collections` for one reason — the arena lives there — and that gravity drags `system → collections` and `usrlib → collections`, which is the exact door through which user-space reads kernel object-identity internals (`apps/shell` prints `.index` / `.generation`).

Consequences we are fixing:

1. **`Handle` is a kernel domain model** (a capability token), not a property of a generic collection. It should be owned by the subsystem that allocates and validates it.
2. **User-space holds no secrets.** A real OS hands out opaque tokens; today user-space can decompose kernel identity structure. With kernel-owned types, opacity becomes *structural*: the kernel is an executable crate — `usrlib` cannot import from it even by mistake. The only thing crossing the boundary is the packed u32 that is already the sole contract, so the kernel can later evolve internals (wider epoch, ASID, parity bits) **without touching user ABI**.
3. **`system` today owns kernel vocabulary** (the IPC failure-mode enums, the `Future` trait, handle semantics). That is the same inversion one crate up. `system` should contain only the *treaty* between the two worlds: numbers and layouts.
4. The `exec`/`load` `u64::MAX` sentinel (special case R1 of PR #57) exists only because `Handle` can encode impossible values. An opaque user handle cannot — the sentinel dies naturally as an `Option`.

## 2. Final outcome (target shape)

```
BEFORE                                       AFTER
collections (Handle+arena+ERROR_BIT)         kernel   owns: collections (arena), handle domain
   ↑        ↑        ↑                          ↑      newtypes (4 distinct), Future trait,
system (domain enums, Future, codec)        system     domain error enums
   ↑        ↑                              system   = pure ABI treaty: SyscallNum,
usrlib, apps (see internals)                usrlib     ErrorCode, ERROR_BIT, Message layout
kernel                                      usrlib   owns: opaque user handle newtypes,
                                                       user error mirrors, user-edge codec
```

- **kernel**: `kernel::collections::arena` (relocated, may become kernel-specific — no generic ceremony needed); `kernel::handle` — four distinct newtypes (task / future / ipc-connection / ipc-binding), private fields, `pack`/`from_packed` only at the syscall edge, 15-bit wrap and domain asserts carried over as kernel policy; `Future` trait (evicted from `system`); kernel-domain error enums that may grow richer than the wire (projection to `ErrorCode` at the edge, implemented kernel-side).
- **system**: numbers and layouts only. `SyscallNum`, `ErrorCode` + `ERROR_BIT` (defined here now — the ABI owner), raw-`usize` classify/extract codec (pure wire arithmetic, names no domain type), `Message { conn: u32, data: [u8; 60] }` + size constants. `system` never names `Handle` or `Future` again.
- **usrlib**: opaque handle newtypes, per-domain user error enums (thin mirrors of `ErrorCode`), `from_reg` at the user edge (`raw → Result<UserHandle, UserError>`), `exec`/`load` returning `Option<UserFutureHandle>`, `Message` built from opaque values. Loses its direct dependency on `collections`.
- **apps**: unchanged except the shell debug print (see R9).
- **collections crate: removed.** Workspace 6 → 5 crates.
- **Translation placement (design law):** a shared crate cannot translate *to* the kernel without depending on it. Therefore translation is implemented **at the two boundary sites** — kernel encodes domain→wire, usrlib decodes wire→user-view — and `system` holds only what both sides may legitimately share. We adopt the *uapi model* (both worlds depend on the treaty crate); the "no shared crate / duplicated constants + golden tests" variant is explicitly deferred until a second or foreign userland exists (see §5).

## 3. Requirements (verifiable)

- **R1 — collections dissolved.** The crate is removed from the workspace; the arena lives under `kernel/` with its unit tests relocated. Kernel behavior unchanged by the move.
- **R2 — `system` names no domain types.** After the refactor, `system/` contains zero references to `Handle`, `GenerationalArena`, `Future` (trait), or `collections`. Grep gate: `use collections` and `generational_arena` → zero matches in `system/`; `Handle` → zero matches in `system/src`.
- **R3 — `ERROR_BIT` moves to `system`** (wire ABI owner). The kernel handle layout is expressed against it; all disjointness law tests survive, relocated. Supersedes D2/D4.
- **R4 — kernel domain typing.** Four distinct newtype types kernel-side; fields private; no `index`/`generation` accessor reachable from outside the `kernel` crate. Passing a task handle where an IPC binding is expected becomes a compile error.
- **R5 — user-space opacity.** No application can decompose a handle identity: `usrlib` handle types have private internals; user-visible `Debug` prints the packed value only (hex), never the struct. Kernel-side `Debug` for handles MAY print `{ index, generation }` — kernel-internal debug aid, not ABI.
- **R6 — two edge codecs.** Kernel owns domain→`usize` encoding (incl. `impl From<KernelError> for ErrorCode` projections); `usrlib` owns `usize`→user decoding. The sealed `ErrorCodeOf` machinery from PR #57 is deleted. Each half is unit-tested in its own crate; the composed roundtrip law is verified end-to-end by `ipc_random` through real syscalls.
- **R7 — sentinel retirement.** `exec`/`load` return `Option<UserFutureHandle>`; the `u64::MAX` special-case (PR #57 R1) is removed; kernel may keep emitting the sentinel value or return a dedicated error — user-side decoding is `None` in the failure case either way (exact kernel-side choice is architecture's call).
- **R8 — WIRE FREEZE (binding).** Packed 32-bit layout (bit 31 = `ERROR_BIT`, generation bits 16–30, index bits 0–15), `ErrorCode` numbering, `Message` 64 bytes / `conn: u32` @ offset 0 / align 4, all syscall numbering and semantics: **bit-identical before and after.** This is a source-layout + visibility refactor; the QEMU suite is the guardrail. Any diff in wire behavior is a defect.
- **R9 — test re-architecture.** `apps/shell` prints the packed handle value (debug); `ipc_random`'s assertion is re-architected from numeric generation growth to the stronger property: after generation wrap the **stale handle is rejected** (`NotFound`/error) and a fresh handle works. The test must stop depending on kernel-internal structure.
- **R10 — domain invariants carried over.** The 15-bit generation domain (`debug_assert` + wrap policy + cycle test from `01715f1`/`1698724`) relocates into the kernel newtypes; the boundary decoder (`from_packed` from user-supplied raw words) validates canonicality exactly as `is_handle` does today.
- **R11 — dependency rules hold.** `no_std` everywhere; `system` gains no dependencies; `usrlib` loses `collections`; `kernel` loses the `collections` path dep and gains none. AGENTS.md updated in the same PR (structure map + "kernel deps" rule).
- **R12 — all existing gates green on final tree:** unit suites (counts will re-baseline: kernel gains the ~21 relocated arena tests, `system`'s 12 redistribute — exact counts established by the verifier baseline, not asserted here), `cargo xtask test --skip-integration`, `cargo xtask build`, bare x86_64 + x86_32 builds, QEMU `boot_and_shell` + `ipc_random`, warning fingerprint Δ=0 against a re-established baseline.

## 4. Non-goals / out of scope

- No wire or ABI change of any kind (R8 overrides everything).
- No re-generalization of the arena (it is kernel infrastructure now; resist const-generic ceremony).
- The "no shared crate" purity model (kernel and userland sharing zero code) — deferred; revisit trigger: a second/foreign userland or a machine-checkable ABI table requirement.
- No functional features: no new syscalls, no allocator work, no arch changes.
- `apps` changes limited to the shell debug print (R9).

## 5. Decision record: why the uapi model (Q2a), not the no-shared-crate model (Q2b)

Q2b (kernel owns a private copy of the ABI constants; `system` is the user-side projection; agreement enforced by golden-table tests) buys a dependency graph where kernel and userland share no code. In a statically linked hobby kernel that purity has **no runtime value** — it is an architectural statement — while it replaces compiler-checked agreement with test-suite vigilance, and the drift failure mode is silent ABI corruption. `ipc_random` catches drift on exercised paths fast, but not on unexercised ones. Q2a keeps the machine-checked treaty (one source for the numbers, both worlds depend on it — how Linux uapi headers work) while the part that actually offended us — `system` owning kernel vocabulary — is evicted either way. **Revisit Q2b only when supporting an unmodified third-party userland.**

## 6. Open decisions (⚠ LOCK before/at architecture)

- **OD1 — user-facing error surface.** Thin per-domain mirror enums in `usrlib` (e.g. `IpcSendError`) mapped `From<ErrorCode>`, vs exposing `ErrorCode` directly to apps.
  **Recommendation: mirror enums** — keeps apps' per-domain `match` ergonomics identical to today and honors the duplication the design dialogue accepted; `ErrorCode` stays the wire vocabulary underneath.
- **OD2 — user handle kinds.** Three opaque newtypes (future / connection / binding) vs one single `UserHandle`.
  **Recommendation: three kinds** — prevents the same capability confusion at the user level that R4 fixes kernel-side. Cost: a few more newtypes in `usrlib`.

## 7. Baseline snapshot & guardrails for execution

Baseline facts below were grep-verified on `main` @ `7171686`, 2026-10-11 — same day as this document; the explorer must still re-verify every claim against the tree before architecture (spec-drift lesson, run 20261010-205728):

- `GenerationalArena` consumers: `kernel/src/{ipc/ipc_manager, ipc/mailbox_manager, task, future, task_manager}.rs` only.
- `system` names `collections`/`Handle` in exactly: `ipc.rs` (conn + enums), `error.rs` (codec + `ERROR_BIT` re-export), `future.rs` (`pub type FutureHandle = Handle`).
- `usrlib` deps: `system` + `collections`; imports `from_reg`, `FutureHandle`, `IpcConnectionHandle`, `IpcSendError`, `IpcReceiveError`, `MESSAGE_PAYLOAD_BYTES`, `SyscallNum`.
- `apps` handle-internals access: `apps/shell/src/shell.rs` (`.index`, `.generation`) only.
- `arch/` has no `collections` imports.
- Test counts today: collections 21, system 12, kernel 182, shell 5, xtask total 220.

Execution guardrails (details belong to the architect, not this document): full loop with verifier baseline first; wire-atomic commits in the PR #57 style (no commit boundary may mix the old source layout with a half-moved dependency edge); QEMU gates mandatory; commit subjects bare imperative; AGENTS.md rewrite included; `cargo build --workspace` remains unsupported.
