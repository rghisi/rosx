# Implementation Steps — Fixed 32-bit Handle + Global ErrorCode

> **Implements:** `plans/fixed-size-handle.md` (decisions D1–D7, O1–O4 — locked, not reopened).
> **Baseline:** HEAD `0a65ed7`, clean tree (Context Map verified 2026-10-10).
> **Superseding authority:** where the Context Map's Discrepancy summary contradicts spec §2, the Context Map wins (extra sites: `usrlib/src/syscall.rs:16,22` sentinel unpacks; `kernel/src/syscall.rs:7,94` HalfSize; `scheduler.rs:219,249`, `timer.rs:39,41` HalfSize; `fifo_strategy.rs:57-58` / `mlfq_strategy.rs:101-102` u32 helpers; `system/src/ipc.rs:205,221,230-231` HalfSize-parameterized tests; CI runs `cargo xtask test --skip-integration`, never plain `xtask test`; no CI job builds x86_32).
> **This document is the binding contract.** Where any code fragment here conflicts with a contract statement or a gate, the contract and gate win.

---

## A. Corrected site inventory (spec §2 ∪ Context Map discrepancies)

### A1. `collections/src/generational_arena.rs` (commit 1)
- Delete: `HalfSize` cfg trio `:4-9`, `HALF_BITS` `:11`.
- Change: `Handle` fields `:15-16` → `u16`; `new` `:20`; `pack`/`unpack` `:24-34` → fixed 32-bit layout; `generations: Vec<HalfSize>` `:45` → `Vec<u16>`; `free_slots: VecDeque<HalfSize>` `:46` → `VecDeque<u16>`; `new()` `slot as HalfSize` `:58` → `slot as u16`; `remove()` bump `:96` `wrapping_add(1)` → `(g + 1) & 0x7FFF`.
- Add: `pub const ERROR_BIT: usize = 1 << 31;`, `Handle::is_handle(raw: usize) -> bool`.
- Tests `:111-300`: drop `HalfSize` import `:114`; rewrite `handle_pack_should_encode_index_in_upper_half_and_generation_in_lower_half` `:119-124`, `handle_unpack_should_decode_index_and_generation_from_usize` `:126-133`, `handle_pack_unpack_should_roundtrip` `:135-139` to fixed-layout expectations; add new law tests (see C1.3). All other tests compile unchanged (integer literals coerce to u16).

### A2. `system/src/ipc.rs` (commits 1, 3)
- Commit 1 (mechanical, forced by A1): test module — drop `HalfSize` import `:205`; `message_new_and_accessors_roundtrip` `:221` `Handle::new(255 as HalfSize, HalfSize::MAX)` → `Handle::new(255, 0x7FFF)`; `err_tag_disjoint_from_256_slot_packed_handles_and_roundtrip` `:230-231` `HalfSize` arithmetic → `u16`-typed arithmetic (test stays otherwise intact; full rewrite lands in commit 3).
- Commit 3 (wire flip): `MESSAGE_PAYLOAD_BYTES = 56` `:8` → `60`; `conn: u64` `:13` → `u32`; `Message::new` `:21` `connection.pack() as u64` → `connection.pack() as u32`; `conn()` `:25` unchanged in shape (`Handle::unpack(self.conn as usize)` — now a u32 widening); delete `IPC_ERR_TAG`/`ipc_err`/`ipc_is_err` `:33-41`; rewrite the four `to_reg`/`from_reg` bodies `:50-62,71-81,91-103,123-137` to delegate through `ErrorCode` (commit 2's machinery); rewrite test module `:202-257` (new layout constants + disjointness via `ERROR_BIT`/`is_handle`).
- Unaffected consumers (accessor API stable — do not touch): `apps/random_gen_server/src/main.rs:42`, `kernel/src/ipc/ipc_manager.rs:121`.

### A3. `usrlib/src/syscall.rs` (commit 3)
- `ipc_is_err` sites `:45` (`wait_for_message`), `:87` (`ipc_connect`), `:106` (`ipc_bind`), `:124` (`ipc_send`) → codec decode (C3.2). Import `:7` drops `ipc_is_err`.
- `ipc_disconnect` `:94-101`: two args (`index as usize`, `generation as usize`) → one arg `connection_handle.pack()`.
- **R1 rule — DO NOT TOUCH:** `exec` `:14-17` and `load` `:19-23` keep plain `FutureHandle::unpack(raw)`. They decode the kernel's `u64::MAX as usize` failure sentinel (`kernel/src/syscall.rs:29,83`). Routing them through `from_reg` would silently convert the sentinel into an error-code path. Under the new codec the sentinel unpacks to `{ index: 0xFFFF, generation: 0x7FFF }` — index above every arena (max 1024) — so it fails at `borrow → Error::NotFound`, identical end-behavior to today.
- **DO NOT TOUCH:** `ipc_receive_message` `:131-139` and `ipc_accept_message` `:141-149` keep bare `FutureHandle::unpack(raw)` (making those calls fallible is out of scope, spec §4).
- Pack-arg sites `wait_future:34`, `wait_for_message:41`, `is_future_completed:53` compile unchanged (`pack()` is the same call).

### A4. `kernel/src/syscall.rs` (commits 1, 3)
- Commit 1 (mechanical, forced by A1): delete `use collections::generational_arena::HalfSize;` `:7`; `:94` `IpcConnectionHandle::new(arg1 as HalfSize, arg2 as HalfSize)` → `IpcConnectionHandle::new(arg1 as u16, arg2 as u16)` — pure type rename, the two-arg wire is preserved until commit 3.
- Commit 3 (wire flip): `:94` → `IpcConnectionHandle::unpack(arg1)`; IpcConnect `:88-91` and IpcBind `:100-103` arms collapse to `into_reg(...)`; add `use system::error::into_reg;`.
- **Unchanged:** Exec `:28-29` / LoadElf `:82-83` (`u64::MAX as usize` sentinel — spec §4, R1); WaitFuture `:49-61` (error path keeps `Err(e) => e.to_reg()`; encoding changes via the delegating body, not the call site); IpcSendMessage `:105-111` (same); `:112-113` keep `.pack()` / `unpack(arg1)` (infallible arms).

### A5. `kernel/` test modules (commit 1, mechanical)
- `kernel/src/scheduler/scheduler.rs:219` import `::{Handle, HalfSize}` → `::Handle`; `:249` `fetch_add(...) as HalfSize` → `as u16`.
- `kernel/src/scheduler/timer.rs:39` import → `::Handle`; `:41` `fn handle(index: HalfSize)` → `fn handle(index: u16)`.
- `kernel/src/scheduler/fifo_strategy.rs:57` `fn make_handle(index: u32, generation: u32)` → `(index: u16, generation: u16)` (R3).
- `kernel/src/scheduler/mlfq_strategy.rs:101` same.
- Compile-unchanged (literals coerce to u16; do not touch): `scheduler.rs:503,513,527-529,541-542,553-554,875,910-911`; `timer.rs:42`; `mailbox_manager.rs:139,143,163,171,200,210,221`; `ipc_manager.rs:249-250` (ABA test; `c1.generation + 1` is valid u16 arithmetic), `:327`; `bitmap_chunk_allocator.rs:706,726,742,743,804`.
- `apps/shell/src/shell.rs:134,149` — field prints and `ipc_disconnect(handle)` call — compile unchanged.

### A6. New / wiring files
- `system/src/error.rs` — NEW (commit 2): `ErrorCode`, `ERROR_BIT` re-export, sealed trait, `into_reg`, `from_reg`, full test suite.
- `system/src/lib.rs` — add `pub mod error;` after `pub mod ipc;` (`:7`).
- `kernel/`, `usrlib/`, `apps/` public APIs unchanged; `FutureHandle`/`TaskHandle`/`MailboxHandle`/`IpcBindingHandle`/`IpcConnectionHandle` aliases unaffected.

---

## B. Interfaces to expose (contracts; coder implements bodies freely within them)

### B1. `collections::generational_arena` (commit 1)
```rust
pub const ERROR_BIT: usize = 1 << 31;

pub struct Handle { pub index: u16, pub generation: u16 }   // derives unchanged (Copy..Ord)
impl Handle {
    pub fn new(index: u16, generation: u16) -> Self;
    pub fn pack(&self) -> usize;        // ((self.generation as usize) & 0x7FFF) << 16 | self.index as usize
    pub fn unpack(packed: usize) -> Self; // index = packed & 0xFFFF; generation = (packed >> 16) & 0x7FFF
    pub const fn is_handle(raw: usize) -> bool;  // raw & !0x7FFF_FFFF == 0  (bit 31 clear AND fits in 32 bits)
}
```
- Layout is exactly D1's diagram: bit 31 = ERROR, bits 30..16 = generation (15), bits 15..0 = index (16). **Note the index/generation halves swap relative to the current `pack`** — that is intended and wire-atomic in-tree.
- `GenerationalArena::new()` gains `assert!(S <= 65536, "arena exceeds 16-bit index space")` alongside the existing `assert!(S > 0, ...)` (all current arenas ≤ 1024; guards §7 risk).
- `remove()` bump: `self.generations[index] = (self.generations[index] + 1) & 0x7FFF;` — values ≤ 0x7FFF so `+ 1` never overflows u16; the arena can never mint a generation that sets bit 31.

### B2. `system::error` (commit 2)
```rust
pub use collections::generational_arena::ERROR_BIT;   // O3: module owns the re-export

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum ErrorCode { /* variants + codes per table B3 */ }

impl ErrorCode {
    pub const fn to_reg(self) -> usize;              // ERROR_BIT | (self as u32 as usize)
    pub fn from_code(raw: usize) -> ErrorCode;       // TOTAL: match raw & 0x7FFF_FFFF, _ => Unknown
}

mod private { pub trait Sealed {} }
pub trait ErrorCodeOf: private::Sealed {
    fn to_error_code(&self) -> ErrorCode;
}
// impls (in error.rs, importing the four enums from crate::ipc):
//   private::Sealed + ErrorCodeOf for IpcConnectionError, IpcBindingError, IpcSendError, IpcReceiveError

pub fn into_reg<E: ErrorCodeOf>(result: Result<collections::generational_arena::Handle, E>) -> usize;
//   Ok(h) => h.pack();  Err(e) => e.to_error_code().to_reg()

pub fn from_reg(raw: usize) -> Result<collections::generational_arena::Handle, ErrorCode>;
//   if raw & ERROR_BIT != 0 -> Err(ErrorCode::from_code(raw))
//   else if Handle::is_handle(raw) -> Ok(Handle::unpack(raw))
//   else -> Err(ErrorCode::Unknown)          // 64-bit garbage with bit 31 clear
```
`from_reg` proves **syntactic well-formedness only**; staleness stays the arena's job (`borrow` → `Error::NotFound`). The `u64::MAX` sentinel has bit 31 set, so it decodes to `Err(Unknown)` — which is precisely why R1 keeps `exec`/`load` off this path.

### B3. ErrorCode variant/code table (binding — names and codes verbatim)
| variant | value | replaces |
|---|---|---|
| `IpcServerNotFound` | `0x0101` | IpcConnectionError::ServerNotFound (1) |
| `IpcConnectionCannotBeEstablished` | `0x0102` | IpcConnectionError::ConnectionCannotBeEstablished (2) |
| `IpcAlreadyBound` | `0x0103` | IpcBindingError::AlreadyBound (1) |
| `IpcSendConnectionNotFound` | `0x0104` | IpcSendError::ConnectionNotFound (1) |
| `IpcSendConnectionCongested` | `0x0105` | IpcSendError::ConnectionCongested (2) |
| `IpcReceiveConnectionNotFound` | `0x0106` | IpcReceiveError::ConnectionNotFound (1) |
| `IpcReceiveNoMessagesAvailable` | `0x0107` | IpcReceiveError::NoMessagesAvailable (2) |
| `IpcMailboxNotAvailable` | `0x0108` | IpcReceiveError::MailboxNotAvailable (3) |
| `Unknown` | `0x7FFF_FFFF` | (new; total-decode catch-all) |

`0x02xx` (tasks/memory) is reserved-but-unpopulated (Context Map §7). **Because AGENTS.md forbids comments, the reservation is documented only in this plan file — do not add placeholder variants or comment markers to reserve it.**

### B4. Domain-enum delegation (commit 3 bodies; signatures public-API-invariant)
```rust
impl IpcConnectionError {
    pub fn to_reg(self) -> usize { self.to_error_code().to_reg() }
    pub fn from_reg(raw: usize) -> Self {
        match ErrorCode::from_code(raw) {
            ErrorCode::IpcConnectionCannotBeEstablished => Self::ConnectionCannotBeEstablished,
            _ => Self::ServerNotFound,
        }
    }
}
```
The other three follow the same shape, preserving today's catch-all defaults (`IpcBindingError`: `_ => AlreadyBound`; `IpcSendError`: Congested on `IpcSendConnectionCongested`, else ConnectionNotFound; `IpcReceiveError`: the two named mappings, else ConnectionNotFound). **Drop `const` from the inherent `to_reg`s** (delegation crosses a trait method; Context Map §6 confirms every `to_reg` call site — `kernel/src/syscall.rs:57,90,102,109`, `usrlib/src/syscall.rs` via `from_reg`, `system/src/ipc.rs:238-255` — is a runtime call, no const context). Coder: grep `to_reg` once more before finalizing to confirm no new const use exists.

### B5. usrlib decode-site forms (commit 3; D7)
```rust
// ipc_connect (raw carries a handle on success):
match from_reg(raw) { Ok(handle) => Ok(handle), Err(_) => Err(IpcConnectionError::from_reg(raw)) }
// ipc_bind: same shape with IpcBindingError.
// wait_for_message / ipc_send (raw is 0 on success, never a handle — tag-test only):
if from_reg(raw).is_err() { Err(IpcReceiveError::from_reg(raw)) } else { Ok(envelope) }
```
`from_reg(0)` is `Ok` (0 is a well-formed packed handle), so the success paths stay success. This is the precise, complete form of D7's "four blocks become `from_reg(raw)`".

---

## C. Steps, grouped per O4's three commits

Gate legend: **[CI-proven]** = command form CI runs on this tree (ci.yml:31,51). **[unverified]** = exact form documented in an archived plan/Context Map but not executed this session (I run no commands); the orchestrator's baseline pass on pristine HEAD must run it green before step C1.1 (per the precedent note in `plans/ipc-capability-handle.md:159`); it is not binding until that transcript exists.

Working directory for all commands: workspace root, except where a step says otherwise.

### Commit 1 — `collections` fixed layout (R2 resolution lives here)

> **R2 sequencing resolution (binding):** NO transitional `HalfSize` alias. Commit 1 deletes `HalfSize` outright and, in the same commit, applies the *pure type-renames* the deletion forces on its consumers: `as HalfSize` → `as u16` and `u32`-typed test helpers → `u16`. These edits change zero wire semantics: `kernel/src/syscall.rs:94` keeps the two-arg disconnect construction, `timer.rs`/`scheduler.rs` handle factories produce identical values, the u32 helpers' literals all fit u16. The wire *encoding* does flip in this commit (pack/unpack swap halves + 32-bit shape), which is safe because every producer/consumer of packed values is in-tree and rebuilt together (spec §2 wire-atomicity argument) — kernel and usrlib both call the shared `collections` codec, so no split tree ever mixes formats. The old `IPC_ERR_TAG` stays disjoint on x86_64 during the 1→3 window (bit 56 vs values ≤ 0x7FFF_FFFF); the transient x86_32 bit-24 proximity window is accepted and risk-listed (F5) — it has no runtime gate today and closes at commit 3. This kills the "final grep gate vs earlier placement" contradiction by construction: `grep HalfSize` is empty from commit 1 onward.

**C1.1 — `collections/src/generational_arena.rs`: layout + tables + wrap.**
Apply A1: delete cfg trio/`HALF_BITS`/`HalfSize`; add `ERROR_BIT`; `Handle` u16 fields; new `pack`/`unpack` (B1 — halves swapped vs today); `is_handle`; `generations: Vec<u16>`; `free_slots: VecDeque<u16>`; `new()` `slot as u16` + `S <= 65536` assert; `remove()` `(g + 1) & 0x7FFF`. `borrow`/`borrow_mut`/`add`/`replace` bodies unchanged (they go through `handle.index as usize`).

**C1.2 — consumer type-renames (same commit; mechanical only).**
- `kernel/src/syscall.rs`: delete `:7` import; `:94` casts → `as u16`.
- `kernel/src/scheduler/scheduler.rs`: `:219` import `::Handle`; `:249` `as u16`.
- `kernel/src/scheduler/timer.rs`: `:39` import `::Handle`; `:41` `fn handle(index: u16)`.
- `kernel/src/scheduler/fifo_strategy.rs:57-58` and `kernel/src/scheduler/mlfq_strategy.rs:101-102`: `fn make_handle(index: u16, generation: u16)`.
- `system/src/ipc.rs` test module per A2 commit-1 line.
Touch nothing else in these files.

**C1.3 — collections test rewrite + new law tests.**
Rewrite the three HalfSize-parameterized tests to fixed expectations (e.g. `Handle::new(5, 10).pack() == (10usize << 16) | 5` — generation upper, index lower; rename the first test to state the fixed 32-bit layout). Add, as separate `#[test]` fns in `mod tests` (no comments; names carry the intent):
- roundtrip over edge handles: `(0,0)`, `(65535,0)`, `(0,32767)`, `(65535,32767)` — `unpack(pack(h)) == h`;
- `pack` never sets bit 31 for any field combination with `generation ≤ 0x7FFF` (iterate boundaries + a stride sweep, e.g. index and generation stepping by 997 — keep runtime small);
- `is_handle`: `0`, `0x7FFF_FFFF` true; `ERROR_BIT`, `ERROR_BIT | 1`, `1usize << 32`, `usize::MAX` false;
- generation cycle: single-slot arena `GenerationalArena<u8, 1>`, loop add/remove 32 768 times, asserting each fresh `add().unwrap().generation` equals the loop counter `& 0x7FFF` and that bit 31 of `pack()` is never set; the generation after `0x7FFF` is `0`, not `0x8000` (spec §5.1, D5).

**C1.4 — commit-1 gates (all green before committing):**
```
cargo test -p collections
cargo test -p system
cargo test -p kernel -- --test-threads=1
cargo build -p kernel --target arch/x86_64/rosx.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec    [unverified — plans/ipc-capability-handle.md:91; compiles cfg(not(test)) syscall.rs without dragging the shell build.rs dep]
cargo xtask test --skip-integration                                                                                                                              [CI-proven]
cargo xtask build                                                                                                                                                [CI-proven]
cargo build --release -p hello_elf -p random_gen_server -p snake -p tetris -p conway --target arch/x86_32/rosx-i686-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec    [unverified — plans/ipc-capability-handle.md:157; MUST run first or the next command's shell build.rs panics]
bash -c 'cd arch/x86_32 && cargo build -p rosx-x86'                                                                                                              [unverified — plans/ipc-capability-handle.md:158; must run from arch/x86_32/ — its .cargo/config.toml supplies rosx-i686.json + build-std; from root it compiles bare-metal asm for the host and dies]
grep -rn "HalfSize" --include='*.rs' .   [gate: zero matches, grep exits 1]
```
**Commit-1 acceptance:** unit suites green with the new collections law tests present (collections count grows from the reported baseline of 15; do not hard-assert totals — see F6); both bare-target builds green; `HalfSize` greps empty. Commit message: plain imperative, no prefix (Context Map §11 style), e.g. `Fixed 32-bit generational arena handle`.

### Commit 2 — `system::error` machinery (purely additive; the wire does not move)

> **Sequencing ruling (binding):** O4 names "wrappers + codec" for commit 2, but flipping the inherent `to_reg`/`from_reg` bodies here would make the kernel (producer) emit bit-31 codes while usrlib (consumer) still checks `ipc_is_err` bit 56 — a half-encoded shared wire, forbidden by O4's closing sentence. Therefore commit 2 is **additive only**: `error.rs` ships `ErrorCode`, the sealed trait, its four impls, the codec, and the full law-suite; the four inherent `to_reg`/`from_reg` bodies, the deletion of `IPC_ERR_TAG`/`ipc_err`/`ipc_is_err`, and all caller flips execute in commit 3's single wire-atomic step. Commit 3 is then purely mechanical, and no commit boundary ever mixes encodings.

**C2.1 — `system/src/lib.rs`:** add `pub mod error;` alongside `pub mod syscall_numbers; pub mod future; pub mod ipc;` (`:5-7`).

**C2.2 — `system/src/error.rs` (new):** implement B2 + B3 exactly. `no_std` legal (zero new deps; `system` already depends on `collections`; Context Map §10). `Unknown = 0x7FFF_FFFF` is a variant whose payload itself decodes as `Unknown` — consistent by construction (`from_code(ERROR_BIT | 0x7FFF_FFFF)` matches it; any other unmatched 31-bit payload also yields `Unknown`). No comments in the file; the `0x02xx` reservation is documented in this plan only (B3).

**C2.3 — `system/src/error.rs` `#[cfg(test)] mod tests`:**
- `from_reg(into_reg(Ok(h))) == Ok(h)` over the same edge-handle set as C1.3;
- for every `ErrorCode` variant `v`: `from_reg(v.to_reg()) == Err(v)`, `to_reg` has bit 31 set, `from_code(v.to_reg()) == v`;
- for every variant of each of the four domain enums `e`: `from_reg(into_reg(Err(e)))` is `Err`, and the enum's `from_reg` is *not yet* delegating — so exercise the trait path directly: `e.to_error_code()` maps to the B3 code, and `from_code(into_reg::<_>(Err(e))) == e.to_error_code()`; the domain-enum roundtrip through inherent `to_reg`/`from_reg` is re-asserted unchanged in commit 3's rewritten tests;
- total decode: `from_code` of garbage payloads (e.g. masked `0x1234`, `0x0001_0000`, and masked `usize::MAX`) is `Unknown`; `from_reg(1usize << 40)` (bit 31 clear, high garbage) is `Err(Unknown)`; `from_reg(u64::MAX as usize)` is `Err(Unknown)` — the R1 sentinel shape;
- sealed-ness is compile-enforced (do not write a test for it; external unimplementability follows from the private-supertrait pattern, O2).

**C2.4 — commit-2 gates:**
```
cargo test -p system
cargo xtask test --skip-integration     [CI-proven — proves kernel/usrlib/apps still compile against the untouched old encoding]
cargo xtask build                       [CI-proven]
cargo build --release -p hello_elf -p random_gen_server -p snake -p tetris -p conway --target arch/x86_32/rosx-i686-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec    [unverified]
bash -c 'cd arch/x86_32 && cargo build -p rosx-x86'                                                                                                              [unverified]
```
**Commit-2 acceptance:** system suite green including the full law-suite; old `ipc_err`-based tests in `ipc.rs` still green (nothing deleted yet); both arches build. Commit message e.g. `Global ErrorCode namespace and register codec in system`.

### Commit 3 — wire-atomic flip (Message + dispatcher + usrlib + grep gates; never split)

**C3.1 — `system/src/ipc.rs`:** A2 commit-3 line — `MESSAGE_PAYLOAD_BYTES = 60`; `conn: u32`; `new`/`EMPTY` per A2; delete `IPC_ERR_TAG`/`ipc_err`/`ipc_is_err`; rewrite the four `to_reg`/`from_reg` bodies per B4 (keep `Display` impls untouched); rewrite the test module: `Message` size 64 / **align 4** / `offset_of!(Message, conn) == 0` / `offset_of!(Message, data) == 4`; `message_new_and_accessors_roundtrip` at extreme `Handle::new(0xFFFF, 0x7FFF)` plus payload pattern fill; disjointness test — `Handle::new(0xFFFF, 0x7FFF).pack() == 0x7FFF_FFFF`, `!Handle::is_handle(ERROR_BIT)`, `is_handle(0x7FFF_FFFF)`, and to_reg/from_reg roundtrips for all four enums through the delegating bodies (spec §5.1–5.2).

**C3.2 — `usrlib/src/syscall.rs`:** A3 — imports (drop `ipc_is_err` from the `system::ipc` use-list; add `use system::error::from_reg;`), four decode sites per B5, `ipc_disconnect` single packed arg. **R1:** `exec`/`load` and the two `*_message` unpacks stay byte-identical.

**C3.3 — `kernel/src/syscall.rs`:** A4 commit-3 line — `use system::error::into_reg;`; IpcConnect/IpcBind arms → `into_reg(services().ipc_manager.borrow_mut().connect(...))` / `.bind_service(...)`; `:94` → `let connection_handle = IpcConnectionHandle::unpack(arg1);`. Everything else untouched (sentinel arms, `.pack()` arms, `to_reg()` error returns — their encoding changes via B4 delegation, not the call site).

**C3.4 — commit-3 + final gates:** Section E, in full, in order. Commit message e.g. `Wire-atomic fixed 32-bit handle codec across kernel, usrlib and Message`.

---

## D. Blast radius: shared-state / transitively-affected test processes

- `kernel/src/syscall.rs` is `cfg(not(test))` — no unit test executes the dispatcher; its proof is the bare-target build gates + QEMU integration. Do not assume the unit gate covers C3.3.
- `Message` is passed across the syscall boundary by pointer (`kernel/src/syscall.rs:55` `ptr::write`, `:106` `&*(arg1 as *const Message)`) and its size/align change in C3.1 → every process kind that moves an envelope must be re-proven with the *same* tree: host unit tests (`--workspace` compiles usrlib+apps on host), bare-target builds (both arches), and QEMU integration (the only process that crosses the real ring-3 boundary). C3.4 mandates all of them.
- `cargo test -p kernel -- --test-threads=1` reaches the scheduler/timer/ipc test modules that C1.2 edits (FakeTaskManager handle factory, timer `handle()` helper, ABA assertion) — proven at commits 1 and 3, not only at the end.
- `tests-integration/tests/ipc_random.rs` exercises the full connect→send→receive→generation-wrap path end-to-end on x86_64; its assertion (`g.1 > first_generation`, Context Map §9 quote) is a numeric comparison — encoding-independent — but it is the *only* runtime proof of the flipped wire.
- App builds (`cargo xtask apps` inside `xtask test`/`build`) recompile `shell` (which prints `.index`/`.generation` and calls `ipc_disconnect`) and `random_gen_server` (which calls `envelope.conn()`) against every commit's API — included in every commit gate via `xtask`.

---

## E. Final acceptance transcript (canonical order: cheap → expensive)

Baseline precondition: on pristine `0a65ed7`, the orchestrator's baseline pass has proven the two `[unverified]` command families (x86_32 pair; integration stems) green *before* commit 1, so each later failure is attributable.

```
# 1-3. package suites
cargo test -p collections
cargo test -p system
cargo test -p kernel -- --test-threads=1

# 4-5. canonical CI unit + build gates
cargo xtask test --skip-integration
cargo xtask build

# 6-7. x86_32 parity (manual — CI has no x86_32 job; do NOT skip; order load-bearing)
cargo build --release -p hello_elf -p random_gen_server -p snake -p tetris -p conway --target arch/x86_32/rosx-i686-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec
bash -c 'cd arch/x86_32 && cargo build -p rosx-x86'

# 8-9. QEMU integration (expensive step-gates; need qemu-system-x86_64 on PATH; scripted harness — not manual)
cargo test -p tests-integration --test boot_and_shell
cargo test -p tests-integration --test ipc_random
#    [unverified stems (plans/ipc-message-api.md:209); if the harness kernel build trips shell/build.rs, run `cargo xtask apps` first;
#     single fallback: `cargo xtask test` runs apps + all unit + both integration tests]

# 10-12. grep gates — each must report ZERO matches (grep exits 1); run from workspace root
grep -rnE "IPC_ERR_TAG|ipc_is_err|ipc_err\(" --include='*.rs' .
grep -rn "HalfSize" --include='*.rs' .
grep -rnE "conn.*u64|u64.*conn" --include='*.rs' .
```

Warnings posture: no warning-baseline fingerprint is recorded in the Context Map, so no absolute warning gate is asserted; the standard cargo output must show **zero new warnings relative to the baseline-pass transcript** (Δ-new = 0; decreases at legitimately deleted sites — e.g. the old `to_reg` consts — are expected, not violations).

---

## F. Risks / pitfalls for the coder

1. **Generation mask is load-bearing everywhere.** `remove()` must be `(g + 1) & 0x7FFF`; `pack()` masks `(gen & 0x7FFF) << 16`; `unpack()` masks `(raw >> 16) & 0x7FFF`. Forgetting any one lets a value set bit 31 and a handle is silently decoded as an error (spec §7.1). The C1.3 cycle + bit-31-law tests are the mitigation — do not skip them.
2. **Halves swap.** New `pack` puts **generation upper, index lower** — the opposite of today's `((index << HALF_BITS) | generation)`. Anyone "fixing" a test by reusing the old formula breaks D1. Expect `ipc_random` shell decimals for generation to change width/values; the wrap assertion is numeric and survives (Context Map §9: no exact-generation string matching anywhere in the file).
3. **R1 sentinel rule.** `exec`/`load` (usrlib `:16,:22`) must keep plain `unpack`; the `u64::MAX` sentinel must never enter `from_reg`. Its post-D1 fate — `{index: 0xFFFF, generation: 0x7FFF}` → `borrow` → `NotFound` — is identical end-behavior to today.
4. **`from_reg` is total but not permissive-with-high-bits:** bit 31 clear + bits 32..63 set ⇒ `Err(Unknown)` via `is_handle`. No in-tree kernel return does this (Context Map §8: all returns zero-extend from u32 post-D1); if a future one does, that is a real ABI bug surfacing, not noise.
5. **Transient x86_32 bit-24 window (commits 1→3).** With the layout flipped but `IPC_ERR_TAG = 1 << 24` still live on x86_32, a handle with generation ≥ 256 would false-positive `ipc_is_err` on an x86_32 image built at those boundaries. No x86_32 runtime gate exists (no CI job; nobody boots intermediate commits). It closes at commit 3. Do not "fix" it early — that would pre-empt the atomic flip.
6. **Test-count baselines:** prior runs reported kernel 199 / collections 15 unit tests (reported, not re-counted here). Commit 1 grows collections (new law tests) and rewrites 3 existing ones; kernel/system counts shift from edits/renames. Assert zero failures + presence of the new named tests; **never hard-assert old totals** (count-equality is unsatisfiable by construction once tests are added/renamed).
7. **`to_reg` loses `const`** (B4). Verify no const-context call sites crept in since the Context Map survey.
8. **x86_32 gate order is load-bearing:** the i686 user-app build must precede `cd arch/x86_32 && cargo build -p rosx-x86`, or `shell/build.rs` panics (missing embedded ELFs under `target/rosx-i686-user/release/`). If the x86_32 user-app build fails *pre-existing* on baseline, shrink the gate to `rosx-x86` alone (shell still type-checks as its dependency — the load-bearing check) and record the shrink in the transcript (precedent: `plans/ipc-capability-handle.md:159`).
9. **Payload-size change ripple:** `MESSAGE_PAYLOAD_BYTES` 56→60 is referenced via the constant everywhere in-tree (usrlib `ipc_send`, apps); coder must grep for any literal `56` sized against `Message` payload before closing commit 3 (none recorded in the Context Map — treat as a check, not a known site).
10. **No comments** in any Rust touched file (AGENTS.md CRITICAL); assembly untouched.

## G. Open items I could not close in-plan

- **`[unverified]` commands** (E#6-9 and the C1.4 kernel-lib bare build): green verbatim status rests on the orchestrator baseline pass, per the gate legend. The plan is not "complete green" until that transcript exists.
- **x86_32 host-toolchain availability** (rust-src pinned nightly-2026-10-03) — assumed present per Context Map §9/§10; if the baseline pass cannot run E#6-7, that gate degrades exactly per F8 and must be recorded, not silently dropped.
- **Integration harness first-run cost** (kernel rebuilt inside `kernel_build.rs`): budget time, not correctness; fallback command recorded at E#8-9.
