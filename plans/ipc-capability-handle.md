# Implementation Plan: Kernel-Mediated Capability Handle IPC for RosX

> **Date:** 2026-10-08 · **Status:** Approved, not started
> **Goal:** Arbitrary-size IPC payloads via kernel-owned buffers referenced from messages by fixed-size generational handles; zero raw pointers inside `IpcMessage`.

## Locked decisions

- **D1 — Uniform buffer-only payloads.** `IpcMessage` carries only an `IpcBufferHandle`; no tagged `Inline|Buffer` enum. `shell` + `random_gen_server` (the only IPC call-sites) migrate to the handle API, keeping ergonomics via usrlib conveniences `ipc_send_value(conn, usize)` / `ipc_receive_value(conn)` implemented as client-side alloc+write+send / receive+read compositions.
- **D2 — Capabilities only (generation-checked).** No owner `TaskHandle` on buffers; any holder of a generation-valid handle may operate per the state machine; freeing is explicit-dispose only. Leaks on abandoned handles are accepted in v1.
- **D3 — Sequential cursor.** Kernel tracks write/read positions; syscalls are `write(handle, ptr, len)` / `read(handle, ptr, len)`; read returns bytes-copied. No explicit offsets (3-register syscall ABI).
- **D4 — Unbounded `Vec`, no per-buffer cap.** Payload grows on demand from the kernel heap; slot pool = `GenerationalArena<MessageBuffer, 256>`.
- **D5 — RAII alongside existing usrfaces.** usrlib gains `WritableMailbox` (write, send consumes, Drop disposes if unsent) and `ReadableMailbox` (read, Drop disposes).

## 0. Validated baseline (all re-verified against the working tree)

- `system/src/ipc.rs:49-53`: `#[derive(Debug, Copy, Clone, PartialEq, Eq)] pub struct IpcMessage { pub data: usize, pub connection_handle: IpcConnectionHandle, }` — the only payload field. `pub type IpcConnectionHandle = Handle;` (l.4-5) is the type-alias idiom to copy.
- `collections/src/generational_arena.rs`: `pack()`/`unpack()` round-trip (l.24-34); `remove` does `self.generations[index].wrapping_add(1)` (l.96) → stale-handle rejection is free; full arena → `Error::OutOfMemory` (l.70).
- `kernel/src/ipc/mailbox_manager.rs:24`: `mailboxes: GenerationalArena<Mailbox, 256>` — the 256 constant to mirror. Tests at l.102-197 construct `IpcMessage { connection_handle: Handle::new(1, 1), data: 123 }` (l.130-133), `data: 456` (l.153-156), `data: 789` (l.189-192) and assert `.result().unwrap().data` (l.136, l.163) — **these are the only kernel literals to migrate**.
- `kernel/src/syscall.rs:14`: `pub fn handle_syscall(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize` — exactly 3 args, and both arch entries pass 3 (`arch/x86_64/src/cpu.rs:175`, `arch/x86_32/src/cpu.rs:108`); both usrlib stubs (`usrlib/src/arch/x86_64.rs:1`, `usrlib/src/arch/x86_32.rs:1`) already accept `arg3` → **zero arch/stub changes needed for the new syscalls** (max 3 args each, verified below).
- `handle_syscall` is `#[cfg(not(test))]` → host tests never type-check its body; every syscall-step needs a bare-metal-target build gate.
- `system/src/syscall_numbers.rs`: highest is `IpcSendToClient = 17`; `TryFrom` match must gain arms in lockstep with the `repr(usize)` variants.
- Call-site census (verified by grep): IPC users are exactly `apps/shell/src/shell.rs` `fn random()` (l.130-160; inner loop `for i in 1..2` runs once; outer `for i in 1..260` runs 259×) and `apps/random_gen_server/src/main.rs` (l.37-43: `let value = self.next() as usize; let _ = Syscall::ipc_send_to_client(msg.connection_handle, value);`). `tests-integration/` is keyboard-token mapping only — no IPC. snake/tetris/conway/hello_elf/test_suite — no IPC.
- Build-graph facts that gate command ordering: `rosx` (x86_64 kernel, `arch/x86_64/Cargo.toml:11`) and `rosx-x86` (`arch/x86_32/Cargo.toml:9`) both depend on the `shell` **lib**; `shell/build.rs` and `arch/x86_64/build.rs` `include_bytes!`/`panic` unless `cargo xtask apps` (x86_64) or the i686-user build (x86_32) ran first. `xtask/src/main.rs:7-11` UNSTABLE flags: `"-Zbuild-std=core,alloc,compiler_builtins"`, `"-Zbuild-std-features=compiler-builtins-mem"`, `"-Zjson-target-spec"`. CI's exact test command (`.github/workflows/ci.yml:31`) is `cargo xtask test --skip-integration`. The kernel **lib** builds standalone for the bare target (no build.rs), enabling an intermediate gate that skips shell.

---

## 1. Binding resolutions (follow verbatim)

**BR1 — State machine: collapse InTransit and ReadOnly into one sealed state (recommended and locked).** Two states only:

```
alloc ──▶ WriteOnly ──seal (inside ipc_send / ipc_send_to_client)──▶ ReadOnly ──dispose──▶ slot freed, generation++
```

| Op (all validate generation first via arena `borrow`/`remove`; stale ⇒ `BufferNotFound`) | WriteOnly | ReadOnly |
|---|---|---|
| `write` | append bytes, `Ok(())` | `Err(Sealed)` |
| `seal` (internal, kernel-only) | transition → ReadOnly, `Ok(())` | `Err(Sealed)` (this rejects double-send of one buffer) |
| `read(dst)` | `Err(Unsealed)` | copy `min(dst.len(), payload.len() − read_pos)`, advance `read_pos`, `Ok(bytes_copied)` |
| `dispose` | free, `Ok(())` | free, `Ok(())` |
| `alloc` | fresh slot in WriteOnly; arena full ⇒ `Err(PoolExhausted)` | — |

Justification: D2 already removed receiver identity, so an `InTransit` state cannot be policed against anyone but the sender reading its *own* sent buffer (self-harm, zero capability value); enforcing it would force `MessageBufferManager` into **both** mailbox dequeue points in `mailbox_manager.rs` (`pop_front_async` l.64 and `notify_waiters` l.87), coupling two arenas and two subsystems. Collapsing keeps `MailboxManager` completely untouched by this change and preserves the pending-future-wakes-same-handle contract unchanged (seal happens *before* `push_back`, so a woken reader always finds the buffer readable — never `Unsealed`-racy).

**BR2 — Bounds are inexpressible by construction and must not be a runtime error path.** With the D3 sequential cursor, writes only append (unbounded Vec per D4) and reads clamp to remaining bytes returning the copied count; an exhausted `ReadOnly` buffer read returns `Ok(0)` (EOF), not an error. The only typed failures are generation (stale) and state violations. Zero-length write → `Ok(())`; zero-cap read → `Ok(())`/0 in both states per BR1's state check order (state checked before clamping).

**BR3 — ABI (locked).** `handle_syscall` supports exactly 3 args; layout per call:

| SyscallNum | arg1 | arg2 | arg3 | return (boxed-Result convention, user deboxes) |
|---|---|---|---|---|
| `IpcBufferAlloc = 18` | — | — | — | `*mut Result<IpcBufferHandle, IpcBufferError>` |
| `IpcBufferWrite = 19` | `buffer_handle.pack()` | `bytes.as_ptr() as usize` | `bytes.len()` | `*mut Result<(), IpcBufferError>` |
| `IpcBufferRead = 20` | `buffer_handle.pack()` | `dst.as_mut_ptr() as usize` | `dst.len()` | `*mut Result<usize, IpcBufferError>` (bytes copied) |
| `IpcBufferDispose = 21` | `buffer_handle.pack()` | — | — | `*mut Result<(), IpcBufferError>` |
| `IpcSend = 13` **repurposed** | `connection_handle.index as usize` | `connection_handle.generation as usize` | `buffer_handle.pack()` (**was** raw `value`) | `*mut Result<(), IpcSendError>` (unchanged) |
| `IpcSendToClient = 17` **repurposed** | same | same | same | same |

Connection handles keep the existing split index/generation encoding (`syscall.rs:92`); buffer handles use the single-`usize` `Handle::pack()`/`Handle::unpack()` codec (x86_64: `u32 index<<32 | u32 generation`; x86_32: `u16<<16 | u16` — both fit `usize` args, verified). `Box::into_raw(Box::new(result)) as usize` on return, `*Box::from_raw(...)` in usrlib, exactly as `IpcConnect`/`IpcSend` do today.

**BR4 — Sealing is kernel-internal and ordered.** `IpcManager::send_to_server`/`send_to_client`: (1) validate connection (`ConnectionNotFound` first — a failed connection check must leave the buffer `WriteOnly` so the caller can dispose/retry), (2) `buffer_manager.seal(...)`, mapping failure to the new `IpcSendError::InvalidBuffer(IpcBufferError)`, (3) `push_back` (which notifies waiters). Never expose `seal` through a syscall.

**BR5 — Ownership transfer on send (documented in commit message, per no-comments policy).** `send` consuming the `WritableMailbox` transfers dispose-responsibility to the receiver; `ReadableMailbox::Drop` fulfills it. Leaks accepted in v1 (D2): buffers abandoned by direct-syscall users, buffers still queued when a peer dies, and everything held by tasks whose `handle_termination` (`scheduler.rs:166-172`) reclaims nothing.

**BR6 — Server MUST dispose request buffers.** The buffer pool is 256 slots global; shell's `random()` allocates 2 buffers per iteration × 259 iterations. If the server ignored request buffers, the pool exhausts at ~iteration 257 (`PoolExhausted` visible in QEMU). The migrated server therefore wraps every received `msg.buffer_handle` in a `ReadableMailbox` binding that drops at loop-scope end.

**BR7 — Directional convenience names.** D1 names `ipc_send_value(conn, usize)` / `ipc_receive_value(conn)` verbatim (client→server / server→client). The server's reply direction additionally gets `ipc_send_value_to_client(conn, usize)` (composition over `ipc_send_to_client`) and `WritableMailbox::send_to_client(conn)` — the minimal symmetric complement; without it the server migrates off RAII onto a raw 5-step sequence, contradicting D1's ergonomics rationale.

---

## 2. Implementation steps

### Step 1 — system crate: handle type, errors, IpcMessage shape, syscall numbers
**Files:** `system/src/ipc.rs`, `system/src/syscall_numbers.rs`
- `ipc.rs`: beside the existing aliases (l.4-5) add `pub type IpcBufferHandle = Handle;`.
- Add `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum IpcBufferError { BufferNotFound, Sealed, Unsealed, PoolExhausted }` + `Display` impl: `"Buffer not found"`, `"Buffer is sealed"`, `"Buffer is not sealed"`, `"Buffer pool exhausted"` (style mirrors `IpcSendError`'s Display at l.23-30).
- `IpcSendError`: extend derive to `#[derive(Debug, Clone, PartialEq, Eq)]` (tests in Step 4 need `assert_eq!`) and add variant `InvalidBuffer(IpcBufferError)` + Display arm `write!(f, "Invalid buffer: {}", e)`. Keep `ConnectionCongested` untouched. Add `impl From<IpcBufferError> for IpcSendError { fn from(e: IpcBufferError) -> Self { Self::InvalidBuffer(e) } }` so usrlib compositions use `?`.
- `IpcMessage`: replace `pub data: usize` with `pub buffer_handle: IpcBufferHandle` (keep `connection_handle` and all derives — `Handle` is `Copy + Eq`). `IpcMessageFuture`, `FutureResult::IpcMessage(Result<IpcMessage, IpcReceiveError>)` (`system/src/future.rs:9`) are shape-agnostic — do not touch.
- `syscall_numbers.rs`: add variants `IpcBufferAlloc = 18, IpcBufferWrite = 19, IpcBufferRead = 20, IpcBufferDispose = 21` **and** the four matching `TryFrom` arms (`18 => Ok(Self::IpcBufferAlloc), …`); omission compiles but silently `Err(())`-rejects the syscalls, so the pair is one edit.

**Why first:** system is a leaf; everything downstream names these types.
**Verify:** `cargo check -p system`
**Known-red after this step (intentional, restored by later steps):** kernel (`syscall.rs:93-96,115` construct `IpcMessage { data: … }`; mailbox tests use `data:`), shell (`received.data` at `shell.rs:145`). usrlib, random_gen_server, and the other apps stay green (they never name the field).

### Step 2 — kernel: mechanical `IpcMessage` shape + send-arm repurposing
**Files:** `kernel/src/syscall.rs`, `kernel/src/ipc/mailbox_manager.rs`
- `syscall.rs`: import `IpcBufferHandle` alongside `IpcConnectionHandle, IpcBindingHandle` (l.9). `IpcSend` arm (l.90-99): `let buffer_handle = IpcBufferHandle::unpack(arg3);` then `IpcMessage { buffer_handle, connection_handle }` — rest unchanged (still `Box::into_raw(Box::new(result)) as usize`). `IpcSendToClient` arm (l.113-118): same substitution of `data: arg3` → `buffer_handle: IpcBufferHandle::unpack(arg3)`.
- `mailbox_manager.rs` tests: the three literals at l.130-133, l.153-156, l.189-192 become `buffer_handle: Handle::new(7, 1)` / `Handle::new(7, 2)` / `Handle::new(7, 3)` (distinct values; `Handle` already imported at l.4); assertions l.136 and l.163 become `assert_eq!(…result().unwrap().buffer_handle, Handle::new(7, 1))` / `Handle::new(7, 2)`.

**Verify:**
- `cargo test -p kernel -- --test-threads=1` (host; exercises updated mailbox tests; `handle_syscall` body is `cfg(not(test))` so add the target gate)
- `cargo build -p kernel --target arch/x86_64/rosx.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec` (from workspace root; compiles the `cfg(not(test))` syscall body without dragging in the still-broken shell dep — kernel lib has no build.rs).

### Step 3 — kernel: `MessageBuffer` state machine + `MessageBufferManager` pool
**Files (new):** `kernel/src/ipc/message_buffer.rs`, `kernel/src/ipc/message_buffer_manager.rs`; **modify:** `kernel/src/ipc/mod.rs` (add `pub(crate) mod message_buffer;` and `pub(crate) mod message_buffer_manager;`).
- `message_buffer.rs`: `pub(crate) enum BufferState { WriteOnly, ReadOnly }`; `pub(crate) struct MessageBuffer { state: BufferState, payload: Vec<u8>, read_pos: usize }` (write cursor is implicitly `payload.len()`; a stored one would be a desync bug). Methods per BR1/BR2: `new() -> Self`, `write(&mut self, bytes: &[u8]) -> Result<(), IpcBufferError>`, `seal(&mut self) -> Result<(), IpcBufferError>`, `read(&mut self, dst: &mut [u8]) -> Result<usize, IpcBufferError>` (state-check first, then clamp-copy, `read_pos += n`).
- `message_buffer_manager.rs`: `pub(crate) struct MessageBufferManager { buffers: GenerationalArena<MessageBuffer, 256> }` (256 per D4, mirroring `GenerationalArena<Mailbox, 256>`), with `new()`, `alloc(&mut self) -> Result<IpcBufferHandle, IpcBufferError>` (`.add(MessageBuffer::new()).map_err(|_| IpcBufferError::PoolExhausted)`), `write(&mut self, handle: IpcBufferHandle, bytes: &[u8]) -> Result<(), IpcBufferError>`, `seal(&mut self, handle: IpcBufferHandle) -> Result<(), IpcBufferError>`, `read(&mut self, handle: IpcBufferHandle, dst: &mut [u8]) -> Result<usize, IpcBufferError>`, `dispose(&mut self, handle: IpcBufferHandle) -> Result<(), IpcBufferError>` (`remove(...).map(|_| ()).map_err(|_| BufferNotFound)`). Pattern: `let buf = self.buffers.borrow_mut(handle).map_err(|_| IpcBufferError::BufferNotFound)?; buf.<op>` — arena staleness and state errors never panic. No notifier, no waiters, no `&'static dyn` port: buffer ops are synchronous and touch neither waiter map (do not conflate with the mailbox→futures map).
- `#[cfg(test)] mod tests` in `message_buffer_manager.rs` (pure `GenerationalArena`/`Vec` code — no services, no notifier needed; unlike mailbox tests these need no `init_services()`), covering exactly the task's list: (1) alloc→write→seal→read happy path asserting content and copied count; (2) cursor accumulation: two writes then split-chunk reads; (3) stale generation: dispose then write/read/seal/dispose on old handle → `BufferNotFound`; (4) ABA: dispose h1(index i, gen g), alloc new handle (same index, gen g+1), ops on stale h1 fail, ops on new handle succeed; (5) write-after-seal → `Sealed`; (6) read-before-seal → `Unsealed`; (7) double dispose → second `BufferNotFound`; (8) seal-twice → second `Sealed`; (9) short read + EOF (`Ok(0)` after drain); (10) zero-size write `Ok`, zero-cap read `Ok(0)`; (11) pool exhaustion: 256 allocs `Ok`, 257th `PoolExhausted`, after one dispose alloc succeeds again.

**Verify:** `cargo test -p kernel message_buffer -- --test-threads=1`

### Step 4 — kernel: `IpcManager` owns the pool, seals on send, exposes passthroughs
**Files:** `kernel/src/ipc/ipc_manager.rs`
- Add field `buffer_manager: MessageBufferManager` to `IpcManager` (after `mailbox_manager`), initialized `MessageBufferManager::new()` in **both** `new()` and `new_with_notifier()`.
- Per BR4, rewrite `send_to_server` (l.114-122) and `send_to_client` (l.124-132): look up connection first (else `Err(IpcSendError::ConnectionNotFound)`); then `self.buffer_manager.seal(message.buffer_handle).map_err(IpcSendError::InvalidBuffer)?;` (disjoint-field borrows of `self.connections` / `self.buffer_manager` / `self.mailbox_manager` keep the borrow checker satisfied); then `push_back` as today. Seal-before-push is required so a future completed by `notify_waiters` can never observe an unsealed buffer.
- Add passthroughs (kernel-private; the syscall layer never touches `MessageBufferManager` directly): `pub(crate) fn alloc_buffer(&mut self) -> Result<IpcBufferHandle, IpcBufferError>`, `pub(crate) fn write_buffer(&mut self, handle: IpcBufferHandle, bytes: &[u8]) -> Result<(), IpcBufferError>`, `pub(crate) fn read_buffer(&mut self, handle: IpcBufferHandle, dst: &mut [u8]) -> Result<usize, IpcBufferError>`, `pub(crate) fn dispose_buffer(&mut self, handle: IpcBufferHandle) -> Result<(), IpcBufferError>`.
- Add `#[cfg(test)] mod tests` to this file using the **existing style of `mailbox_manager.rs` l.140-164** (`init_services();` + `Box::leak(Box::new(FutureRegistryNotifierUseCase { future_registry: services().future_registry })) as &'static dyn ForNotifyingFutures` + `IpcManager::new_with_notifier`; these tests build *local* `IpcManager` instances — the global registry is not touched, so no service-name collisions; `NoopIpcNotifier`'s `register → None` would panic on `.unwrap()` in the receive path, hence the real notifier). Cases: (1) bind+connect+alloc+write → `send_to_server` `Ok`; `receive_from_all_clients_async(binding)` → downcast `IpcMessageFuture` in `services().future_registry`, assert `result().unwrap().buffer_handle == handle`; then `read_buffer` returns exactly the written bytes as `ReadOnly` — proves seal-on-send end-to-end on host; (2) second send of the same handle → `Err(IpcSendError::InvalidBuffer(IpcBufferError::Sealed))`; (3) dispose then send → `Err(InvalidBuffer(BufferNotFound))`; (4) `disconnect` then send → `Err(ConnectionNotFound)` **and** `read_buffer` → `Err(Unsealed)` (proves seal does not run when the connection check fails first).

**Verify:** `cargo test -p kernel -- --test-threads=1` and the Step-2 target-build gate.

### Step 5 — kernel: four new syscall arms (numbers 18-21)
**File:** `kernel/src/syscall.rs`
Add arms (before `Err(_)`) per the BR3 table, using the passthroughs from Step 4. `IpcBufferWrite`/`IpcBufferRead` build the user slice with `unsafe { core::slice::from_raw_parts(arg2 as *const u8, arg3) }` / `from_raw_parts_mut` — consistent with the established convention in this exact file (`Print` arm: `core::str::from_utf8_unchecked(core::slice::from_raw_parts(arg1 as *const u8, arg2))`; justified by no address-space isolation, per Context Map). All four return `Box::into_raw(Box::new(result)) as usize`; `IpcBufferAlloc`/`IpcBufferDispose` use `IpcBufferHandle::unpack(arg1)`; `IpcBufferAlloc` ignores args. TOCTOU note (invariant by construction, single core + IF masked at syscall entry on both arches): the state-check→copy sequence cannot interleave with any other mutator.
**Verify:** `cargo build -p kernel --target arch/x86_64/rosx.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec` + `cargo test -p kernel -- --test-threads=1`.

### Step 6 — usrlib: raw wrappers + repurposed send signatures
**File:** `usrlib/src/syscall.rs`
- Extend the `system::ipc` import (l.5) with `IpcBufferError, IpcBufferHandle`.
- Change `pub fn ipc_send(connection_handle: IpcConnectionHandle, value: usize) -> Result<(), IpcSendError>` → `pub fn ipc_send(connection_handle: IpcConnectionHandle, buffer_handle: IpcBufferHandle) -> Result<(), IpcSendError>`, arg3 becomes `buffer_handle.pack()`; same for `ipc_send_to_client` (l.98-101). Boxed-Result debox unchanged.
- Add four wrappers, one per BR3 row: `pub fn ipc_alloc_buffer() -> Result<IpcBufferHandle, IpcBufferError>`, `pub fn ipc_write_buffer(buffer_handle: IpcBufferHandle, bytes: &[u8]) -> Result<(), IpcBufferError>` (passes `bytes.as_ptr() as usize, bytes.len()`), `pub fn ipc_read_buffer(buffer_handle: IpcBufferHandle, dst: &mut [u8]) -> Result<usize, IpcBufferError>` (passes `dst.as_mut_ptr() as usize, dst.len()`), `pub fn ipc_dispose_buffer(buffer_handle: IpcBufferHandle) -> Result<(), IpcBufferError>`; each deboxes via `unsafe { *Box::from_raw(raw as *mut Result<_, IpcBufferError>) }` per the existing convention (l.67-101).
- No arch changes: `raw_syscall(num, arg1, arg2, arg3)` exists on both targets (verified; gates in Step 9 prove x86_32).

**Verify:** `cargo check -p usrlib`
**Known-red after this step:** `random_gen_server` (`ipc_send_to_client(conn, value)` call, fixed in Step 8); no gate between Steps 6-8 depends on it.

### Step 7 — usrlib: RAII module + value conveniences
**Files:** `usrlib/src/ipc.rs` (new), `usrlib/src/lib.rs` (add `pub mod ipc;`).
Signatures (D5/D1/D7 contracts):
- `pub struct WritableMailbox { buffer: IpcBufferHandle, sent: bool }`
  - `pub fn alloc() -> Result<Self, IpcBufferError>`
  - `pub fn write(&mut self, bytes: &[u8]) -> Result<(), IpcBufferError>`
  - `pub fn send(mut self, connection: IpcConnectionHandle) -> Result<(), IpcSendError>` — calls `Syscall::ipc_send`; sets `sent = true` **only on `Ok`**; `pub fn send_to_client(mut self, connection: IpcConnectionHandle) -> Result<(), IpcSendError>` (BR7) — same rule.
  - `impl Drop` — `if !self.sent { let _ = Syscall::ipc_dispose_buffer(self.buffer); }` (covers `Err(ConnectionNotFound)`: buffer was not sealed, dispose succeeds; covers panic-abandonment).
- `pub struct ReadableMailbox { buffer: IpcBufferHandle }`
  - `pub fn new(buffer: IpcBufferHandle) -> Self` (public: the server builds one from a received `msg.buffer_handle`)
  - `pub fn read(&mut self, dst: &mut [u8]) -> Result<usize, IpcBufferError>`
  - `impl Drop` — dispose, `let _ =`.
- `pub fn ipc_send_value(connection: IpcConnectionHandle, value: usize) -> Result<(), IpcSendError>` = `alloc → write(&value.to_ne_bytes()) → send` with `?` (relies on the Step-1 `From<IpcBufferError>`).
- `pub fn ipc_send_value_to_client(connection: IpcConnectionHandle, value: usize) -> Result<(), IpcSendError>` — same via `send_to_client` (BR7).
- `pub enum ReceiveValueError { Receive(IpcReceiveError), Buffer(IpcBufferError), ShortRead }` + `Display`; `impl From<IpcBufferError>` and `From<IpcReceiveError>` for it.
- `pub fn ipc_receive_value(connection: IpcConnectionHandle) -> Result<usize, ReceiveValueError>` = `Syscall::ipc_receive → Syscall::wait_future` (same blocking posture as today's shell) → on `FutureResult::IpcMessage(Ok(msg))`: `ReadableMailbox::new(msg.buffer_handle)`, read into `[0u8; core::mem::size_of::<usize>()]`, `Err(ShortRead)` if the count is short, else `usize::from_ne_bytes`; on `IpcMessage(Err(e))` → `Receive(e)`; any other `FutureResult` arm → `Receive(IpcReceiveError::NoMessagesAvailable)`. The `ReadableMailbox` Drop disposes on every path. `size_of::<usize>()` makes it 8-byte on x86_64 / 4-byte on x86_32 with no `cfg` — sender and receiver always share an arch.

**Verify:** `cargo check -p usrlib`

### Step 8 — apps migrate (the only two IPC call-sites) + full green
**Files:** `apps/random_gen_server/src/main.rs`, `apps/shell/src/shell.rs`
- Server loop (l.38-44): inside `if let FutureResult::IpcMessage(Ok(msg)) = Syscall::wait_future(fh)`, first bind `let _request = ReadableMailbox::new(msg.buffer_handle);` (drops at scope end → disposes the client request; **mandatory** per BR6 pool arithmetic), then keep `let value = self.next() as usize;` and replace the send line with `ipc_send_value_to_client(msg.connection_handle, value)`. Imports: `usrlib::ipc::{ipc_send_value_to_client, ReadableMailbox}`.
- Shell `random()` (l.130-160): keep connect/print/disconnect structure verbatim (including `println!("RANDOM Server: {} {}", ipc_connection.index, ipc_connection.generation)` — `Handle` fields unchanged); replace `Syscall::ipc_send(ipc_connection, 123456)` with `ipc_send_value(ipc_connection, 123456)`; replace the inner receive block (`ipc_receive` + `wait_future` + `received.data`) with `match ipc_receive_value(ipc_connection) { Ok(value) => println!("RANDOM Value: {}", value), Err(e) => println!("RANDOM Value not received: {}", e) }`; keep the `Err(result) => println!("RANDOM Failed to send: {}", result)` arm (works via `IpcSendError: Display`). Imports: `usrlib::ipc::{ipc_send_value, ipc_receive_value}`. The 259× connect/disconnect loop and its connection-arena recycling exercise are preserved.

**Verify (full x86_64 stack green):**
1. `cargo xtask apps` (compiles migrated random_gen_server for `arch/x86_64/rosx-user.json`; also refreshes the ELFs the kernel build.rs embeds)
2. `cargo xtask test --skip-integration` (exact CI command; compiles migrated shell for host as a default-member and runs every unit test)
3. `cargo xtask build` (x86_64 kernel + disk image; `cd arch/x86_64 && cargo build` is the equivalent kernel-only gate)

### Step 9 — x86_32 parity (u16 `HalfSize` packing)
No code change expected; this step proves it.
1. From workspace root (mirrors xtask's UNSTABLE list with the i686 user spec; also refreshes the ELFs `shell/build.rs` requires at `target/rosx-i686-user/release/`):
`cargo build --release -p hello_elf -p random_gen_server -p snake -p tetris -p conway --target arch/x86_32/rosx-i686-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec`
2. `cd arch/x86_32 && cargo build -p rosx-x86` (must run from that directory — its `.cargo/config.toml` supplies `target = "rosx-i686.json"` + build-std; from the root this compiles bare-metal asm for the host and dies).
**Note:** the Context Map's claim that these `-Z` flags appear in CI is **false** — `ci.yml` and `build-artifacts.yml` contain no x86_32 job (verified). Run step 9.1 on the unmodified main branch before starting; if it fails pre-existing, the x86_32 user-app parity gate is out of scope for this change and shrinks to step 9.2 alone (shell still type-checks against the new usrlib as a `rosx-x86` dependency, which is the load-bearing check).

### Step 10 — QEMU end-to-end observation (manual gate)
`cargo xtask build` then `cargo run -p rosx` (run.sh → x86_64-runner → BIOS image → qemu). The RANDOM server auto-starts (`arch/x86_64/src/main.rs:75` schedules the ELF; expect `[IPC Server] Random`). At `rose>` type `random` + Enter. Expected: 259 successive blocks of `RANDOM Server: <index> <generation>` (generation visibly increments as the 256-slot connection arena round-robins) + `RANDOM Value requested` + `RANDOM Value: <n>` with a strictly-changing PRNG sequence, **zero** occurrences of `Value not received`, `Failed to send`, `Connection failed`, or `[IPC Server] Random - Panic!`. A `PoolExhausted`-style send failure mid-run means a dispose path leaked slots (check BR6).

---

## 3. Risks & edge cases the coder must handle

1. **Generation-reuse ABA:** `arena.remove` bumps generation with `wrapping_add(1)` — after 2^32 (x86_64) / 2^16 (x86_32) recycles of the *same slot*, a stale handle can alias a live buffer. 259-iteration shell loop reuses each of the 256 slots ≤ 2×; wrap is unreachable in practice but document via commit message. Test coverage: Step 3 case (4).
2. **Accepted leaks (D2):** abandoned handles, messages never received, terminated tasks — no reclamation, no `TaskHandle` on any IPC object. The RAII types are mitigation, not a guarantee.
3. **x86_32 u16 packing:** `pack()` = `index<<16 | generation`; slot count 256 and HalfSize `u16` verified compatible; but `arg1 as HalfSize` truncation idioms (as at `syscall.rs:86`) must NOT be re-used for buffers — always `Handle::unpack` on the full `usize` (plan prescribes exactly this).
4. **Double-send of one buffer** is the main capability-violation vector — rejected by `Err(Sealed)` via the seal-twice path (Step 3 case 8, Step 4 case 2).
5. **Seal ordering:** sealing after `push_back` would let a woken receiver read a `WriteOnly` buffer — forbidden; BR4 fixes the order, Step 4 test (1) pins it.
6. **Direct-syscall user disposing a queued buffer** (after `send` succeeded, bypassing RAII): receiver gets a delivered message whose `read` → `BufferNotFound` — typed error, never panic; accepted v1 behavior.
7. **`register` returns `None` with the Noop notifier** — IPC receive tests must use the real `FutureRegistryNotifierUseCase` wiring exactly as `mailbox_manager.rs` l.141-145; using `IpcManager::new()` on a receive path panics on `.unwrap()`.
8. **`TryFrom` desync:** forgetting the 18-21 match arms compiles cleanly but makes the new syscalls return `0`; Step 5's target build + Step 10's QEMU run are the only detectors.
9. **Blast radius of the field rename:** `grep -rn "pub data" system/src/ipc.rs` and `grep -rnE "data: [0-9]+" kernel/src/ipc` must be empty at the end; `FutureResult::IpcMessage` consumers (shell, random_gen_server — the only two) are covered by Step 8; `tests-integration` has no IPC (verified) so it cannot silently break.
10. **No comments policy vs. unsafe justification:** AGENTS.md demands a documented reason for every `unsafe`; resolve through the commit message (the repo's existing `unsafe` in `syscall.rs`/kernel_cell carries no in-file comments — keep repo convention).

## 4. Acceptance criteria (all must pass)

```
cargo check -p system
cargo test -p kernel -- --test-threads=1
cargo build -p kernel --target arch/x86_64/rosx.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec
cargo check -p usrlib
cargo xtask apps
cargo xtask test --skip-integration
cargo xtask build
cargo build --release -p hello_elf -p random_gen_server -p snake -p tetris -p conway --target arch/x86_32/rosx-i686-user.json -Zbuild-std=core,alloc,compiler_builtins -Zbuild-std-features=compiler-builtins-mem -Zjson-target-spec
cd arch/x86_32 && cargo build -p rosx-x86
grep -rn "pub data" system/src/ipc.rs                # must print nothing
grep -rnE "IpcMessage \{[^}]*data" kernel/src system/src usrlib/src apps   # must print nothing
grep -rnE "Syscall::ipc_send\([a-z_]+, [0-9]+\)" apps                      # must print nothing
```
Plus the Step-10 manual QEMU gate: full 259-iteration shell↔RANDOM random-generation flow succeeds over buffer payloads with no send failures, no pool exhaustion, and no panics.