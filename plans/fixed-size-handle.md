# Plan — Fixed 32-bit Handle with reserved error bit and global ErrorCode

> **Date:** 2026-10-10 · **Status:** All design decisions settled with user (D1–D7, O1–O4); architect pass pending
> Format follows `plans/ipc-fixed-message.md`. Decisions D1–D7 (§3) and O1–O4 (§6) were **settled in discussion with the user** — the architect must not reopen them; §6 states the rulings the detailed plan must implement.

## 1. Objective

Replace the variable-width `Handle` (usize-derived `HalfSize` halves) with a **fixed 32-bit encoding on every architecture**: 16-bit index, 15-bit generation, bit 31 reserved as an error tag. Make the syscall return register a structurally disjoint sum of "packed handle" and "global error code", decodable by one uniform codec into `Result<Handle, ErrorCode>`. Delete the usize-relative `IPC_ERR_TAG`, the `pack() as u64` widening in `Message`, the two-arg `ipc_disconnect` workaround, and the documented-only invariant that x86_32 bit 24 (the `FutureHandle` proximity flagged in `plans/ipc-fixed-message.md` §3.3) must never be tag-decoded.

## 2. Verified baseline (checked 2026-10-10 against working tree at HEAD 0a65ed7)

- `collections/src/generational_arena.rs`: `HalfSize` cfg trio (l.4-9), `Handle { index, generation: HalfSize }` (l.13-17), `pack`/`unpack` usize codec (l.24-34), `generations: Vec<HalfSize>` (l.45), `free_slots: VecDeque<HalfSize>` (l.46), `remove` bumps generation with `wrapping_add(1)` (l.96).
- Arena capacities: tasks 256 (`kernel/src/task_manager.rs:8`), futures **1024** (`kernel/src/future.rs:82`), mailboxes 256 (`kernel/src/ipc/mailbox_manager.rs:24`), bindings + connections 256 (`kernel/src/ipc/ipc_manager.rs:46,48`). Max live index ≤ 1023 ⇒ fixed 16-bit index has ≥ 64× headroom.
- `system/src/ipc.rs`: `Message { conn: u64, data: [u8; 56] }` (l.10-15), `new`/`conn` widen/truncate through `usize` (l.21,25), `IPC_ERR_TAG = 1 << (usize::BITS - 8)` + `ipc_err`/`ipc_is_err` (l.33-41), four error enums with `to_reg`/`from_reg` (local codes 1..=8), layout tests asserting size 64 / align 8 / offsets (l.207-257).
- `usrlib/src/syscall.rs`: `ipc_is_err` decode sites l.45, 87, 106, 124; `ipc_disconnect` passes `index` and `generation` as two separate args (l.94-101); bare-handle `unpack` on receive/accept returns (l.131-149).
- `kernel/src/syscall.rs`: `pack()` return arms l.28, 82, 89, 101, 112-113; `unpack(arg1)` arms l.50, 63.
- Direct field access sites (must keep compiling if Handle stays a struct): `apps/shell/src/shell.rs:134`, `usrlib/src/syscall.rs:97-98`, `kernel/src/scheduler/timer.rs:98,108`, tests in `kernel/src/ipc/ipc_manager.rs:249-250`.
- Aliases are pure re-exports of `Handle` — unaffected by encoding change: `FutureHandle` (`system/src/future.rs:5`), `IpcConnectionHandle`/`IpcBindingHandle` (`system/src/ipc.rs:4-5`), `TaskHandle` (`kernel/src/task.rs:10`), `MailboxHandle` (`kernel/src/ipc/mailbox_manager.rs:21`), local `IpcBindingHandle` (`kernel/src/ipc/ipc_manager.rs:37`).
- Wire format has no cross-build persistence: every producer/consumer of packed handles is in-tree and recompiled together (kernel, usrlib, apps, tests) ⇒ encoding may change atomically, no migration shim needed.
- Gates: CI runs `cargo xtask test` and `cargo xtask build` (`.github/workflows/ci.yml`); x86_32 kernel builds via `arch/x86_32/.cargo/config.toml` custom target; QEMU integration tests `tests-integration/tests/ipc_random.rs` + `boot_and_shell.rs`.

## 3. Settled design

**D1 — Fixed 32-bit register layout (arch-invariant).**

```
bit  31        30 ......... 16   15 ......... 0
     |          |                |
   ERROR     generation(15)    index(16)
```

`Handle` packs to a `u32`-shaped `usize` with bit 31 always clear. Slots ≤ 65535, generations wrap at 2^15 = 32768. Rationale (agreed): 16-bit total rejected (generation wrap at 64 = ABA-unsafe at web-server rates; 10-bit slot = zero headroom over the 1024 futures arena); 64-bit rejected (re-enshrines the x86_32 ABI divergence this plan removes); 32-bit equals today's x86_32 semantics promoted to a guarantee.

**D2 — Error tag is the absolute constant `ERROR_BIT = 1 << 31`, defined in `collections`.** Handle-space and error-space are disjoint **by construction** on every platform; the "decode tag only in connect/bind/send/wait paths" convention from `plans/ipc-fixed-message.md` is retired. `IPC_ERR_TAG = 1 << (usize::BITS - 8)` and the x86_32 bit-24 `FutureHandle` proximity die with it.

**D3 — Global self-describing `ErrorCode`.** One `#[repr(u32)]` enum in `system`, in a new module `system/src/error.rs` (O3) holding the whole OS error namespace; payload after the tag is the full 31 bits so the value is decodable without knowing the call site. Reserved ranges per subsystem (initial: `0x01xx` ipc — migrate the 8 existing local codes into it — `0x02xx` tasks/memory, `0x7FFF_FFFF = Unknown`). `from_code` is **total**: unknown ⇒ `Unknown` (catches 64-bit register garbage whose bit 31 happened to be set). The four existing error enums keep their public API as thin wrappers delegating to `ErrorCode` (O2).

**D4 — Register codec lives in `system`, split-ownership with `collections`.** `collections` owns layout (`ERROR_BIT`, `Handle::pack`/`unpack`, a `Handle::is_handle(raw)` well-formedness check); `system` owns semantics:

```rust
pub fn into_reg<E: ErrorCodeOf>(result: Result<Handle, E>) -> usize
pub fn from_reg(raw: usize) -> Result<Handle, ErrorCode>
```

Free functions, not `From`/`TryFrom` impls (agreed: implicit ABI-wide `From<...> for usize` is too magical). `from_reg` proves syntactic well-formedness only; staleness stays the arena's job (`borrow` → `Error::NotFound`). The generic bound (`ErrorCodeOf`) is a **sealed trait** — open to callers, closed to implementors outside `system` (O2); the four domain enums keep public inherent `to_reg`/`from_reg` delegating through `ErrorCode`, so app-facing API is unchanged.

**D5 — Arena generation counters wrap at 15 bits.** `remove()` bumps via `(g + 1) & 0x7FFF` so the arena itself can never mint a generation that sets the error bit. `generations: Vec<u16>`, `free_slots: VecDeque<u16>` — `HalfSize` and all three `target_pointer_width` cfgs are deleted from `collections`.

**D6 — `Message.conn` becomes `u32`.** Envelope stays 64 bytes; `MESSAGE_PAYLOAD_BYTES` 56 → **60**; align 8 → 4. The `pack() as u64` / `as usize` dance in `Message::new`/`conn` collapses to direct `u32` store/load. Layout tests updated to the new constants/offsets.

**D7 — Call sites collapse to the codec.** Dispatcher arms returning `{Ok(h) => h.pack(), Err(e) => e.to_reg()}` become `into_reg(result)`; usrlib's four `if ipc_is_err(raw)` blocks become `from_reg(raw)`. `ipc_disconnect` consolidates from two args (index, generation) to one packed arg.

## 4. Scope boundaries — NOT in this plan

- Making `IpcReceiveMessage`/`IpcAcceptMessage` fallible (enabled by this design, deliberately deferred).
- Handles embedded inside message payloads / cap transfer.
- `Exec`/`LoadElf` `u64::MAX` failure sentinel (separate domain, untouched).
- Any arena size change; any change to syscall numbers, mailbox depth, or message routing.
- Apps keep compiling against the same public API (`ipc_connect` etc. keep returning domain enums); no app-visible signatures change.

## 5. Verification requirements

1. Codec roundtrip laws as unit tests: `unpack(pack(h)) == h` over edge handles (index 0/65535, generation 0/32767); `from_reg(into_reg(Ok(h))) == Ok(h)`; `from_reg(into_reg(Err(e))) == Err(e)` for every `ErrorCode` variant; every packed handle has bit 31 clear; generation counter cycles 32767 → 0 (not 32768) without ever setting bit 31.
2. `Message` layout test: size 64, `conn` at offset 0 as u32, align 4, roundtrip at extreme handle.
3. Existing suites green: `cargo xtask test`, kernel `--test-threads=1`, plus bare-target builds for **both** x86_64 and x86_32.
4. QEMU integration unchanged and green: `ipc_random.rs` (its connection-generation-wrap assertion exercises D5's 15-bit counter directly) + `boot_and_shell.rs`.
5. Grep gates: zero `u64` conn / zero `IPC_ERR_TAG` / zero `ipc_is_err` remnants outside the codec; zero `HalfSize` remnants anywhere.

## 6. Settled rulings (O1–O4, user-approved 2026-10-10)

1. **O1 — `Handle` stays a struct** `{ index: u16, generation: u16 }`. `pack`/`unpack` enforce the layout and the bit-31 invariant; every §2 field-access site keeps compiling unchanged.
2. **O2 — sealed trait, public wrappers.** The domain-generic bound used by `into_reg` is a **sealed trait**: `pub` only insofar as callers (kernel dispatcher, usrlib) need it, but unimplementable outside `system` via the standard private-supertrait pattern — the Rust `private_bounds` lint forbids a `pub fn` bounded on a fully private trait, and the kernel is a separate caller. Implemented exactly on the four domain enums (all live in `system/src/ipc.rs`); they keep their public inherent `to_reg`/`from_reg` implemented as delegation through `ErrorCode`. Apps see no new machinery; existing usrlib/kernel call sites compile except where D7 explicitly replaces them.
3. **O3 — codec lives in `system/src/error.rs`.** The module owns `ErrorCode`, the `ERROR_BIT` re-export, `into_reg`, `from_reg` — one auditable file for the whole register ABI (legal under the `no_std` + `collections`/`system`-only rule).
4. **O4 — 3-commit sequence** (each must build both arches): **(1)** `collections`: fixed layout, `u16` side tables, 15-bit generation wrap, codec laws; **(2)** `system`: `error.rs` + `ErrorCode` + private trait + wrappers + codec and layout tests; **(3)** wire-atomic flip — kernel dispatcher arms, usrlib decode sites, `ipc_disconnect` single-arg, `Message.conn` u32 + payload 60 + updated layout tests — plus final grep gates. Commits touching a shared wire never land half-encoded; Message and the dispatcher cannot split (the §2 wire-atomicity argument forbids it).

## 7. Risks

- Forgetting the 15-bit generation mask in `remove()` ⇒ arenas can mint "handles" with bit 31 set ⇒ silently decoded as errors — mitigated by verification item 1.
- A future arena above 65535 slots breaks D1 silently; if ever needed, only the index/generation re-split changes, the 32-bit ABI stays.
- `ipc_random` observes generation values printed by the shell — the printed decimals will differ in width (15-bit values) but the wrap assertion (`g.1 > first_generation`) is encoding-independent; verify no test string-matches exact generation decimals.
