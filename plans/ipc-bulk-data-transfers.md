# IPC Data Transfers — Planning Spec

> **Date:** 2026-10-15
> **Branch:** `ipc-bulk-data-transfers` (from `main` @ `afcb874`)
> **Status:** LOCKED for execution — all decisions resolved with the user 2026-10-15 (§7); no open decisions remain.

---

## 1. Why (motivation)

IPC today exchanges fixed 64-byte `Message`s (`conn: u32` + 60-byte inline payload). Bulk use cases — writing data to a file, sending a network packet — do not fit: a 40 KB transfer costs 683 messages plus one future completion each. Chunking over messages re-implements fragmentation badly in user space; enlarging `Message` caps bulk and multiplies mailbox memory (32 × enlarged slot per connection).

The design chosen separates the two concerns cleanly:

- **Messages** — unchanged. The *only* notification/synchronization mechanism. They coordinate: requests, replies, and **"data is ready for you"**.
- **Data** — connection-scoped, out-of-band byte payloads held kernel-side. Storage operations only, no waiting semantics of their own. Two roles ever: one **writer**, one **reader**, always the two endpoints of an existing point-to-point connection. Data cannot announce itself; a message must carry its handle.

The new syscalls are **complementary** to the existing ones: nothing in the mailbox, connection, binding, or future machinery changes.

## 2. Final outcome (target shape)

```
writer                          kernel                            reader
  ipc_data_write(conn, ptr, len)
  ├─ snapshot copy user bytes ─────► DataObject{payload, conn, writer, cursor}
  ◄─ packed IpcDataHandle token ─┘   (pending; invisible to the mailbox)
  ipc_send_message(conn, payload{token, len})   ← existing syscall; kernel treats payload as opaque
  ├──────────────────────────────► mailbox.push(Message) ──► waiter future completes
                                                       ipc_data_read(token, dst, len) ×N
                                                       (done — protocol ack, no reader release)
  ipc_data_release(token)         ← writer only
  ├──────────────────────────────► payload freed, writer budget refunded
```

Key properties:

1. **Snapshot copy-in.** `ipc_data_write` returns after the bytes are copied into kernel memory; the writer may reuse/mutate its buffer immediately. No lifetime coupling between the peers (the flaw that disqualifies pointer-rendezvous).
2. **The kernel never interprets message payloads.** The data token inside a payload is bytes the two applications agreed on. Token validity, reader identity, and writer identity are checked **once, kernel-side**, inside the data syscalls. Consistent with the ownership principles: user space carries opaque tokens; the kernel is the single authority on invariants.
3. **The wire is frozen.** `Message` stays 64 bytes, layout tests keep passing. No kind byte, no payload shrink.
4. **Works unchanged under future MMU isolation.** All crossings are explicit kernel copies at the syscall edge, so the design survives the day user/kernel address spaces split.
5. **Cost:** a 40 KB transfer ≈ 1 write syscall + 1 accept + N chunk reads + 1 release, exactly two copies (user→kernel, kernel→user).

### Locked design decisions (user-confirmed 2026-10-15)

| Aspect | Decision |
|---|---|
| Syscalls | `IpcDataWrite = 24 (conn, ptr, len) → token`, `IpcDataRead = 25 (token, dst, len) → bytes copied`, `IpcDataRelease = 26 (token)` |
| Read semantics | **Sequential kernel-side cursor.** Returns bytes copied (`min(len, remaining)`), `0` at end. No random access in v1 (3-arg syscall ABI fits cursor exactly) |
| Release | **Writer only.** Reader has no `release()` — "done reading" is purely a protocol ack on top of messages. usrlib `Drop` on the writer handle auto-releases |
| Multiplicity | **Many** live `DataObject`s per connection direction; one message may reference several tokens |
| Payload protocol | **The application decides.** Neither `system` nor the kernel mandates DataRef placement or encoding. `usrlib` ships a canonical convenience (`send_data`/`receive_data`, token at payload offset 0) as a library default, not a treaty requirement |
| Limits | Kernel constants: 64 KiB per DataObject, 256 KiB outstanding per task, arena capacity 256 |
| Cleanup | v1: connection-scoped GC on `disconnect`. Fast-follow: task-death GC of dangling objects (a crashed endpoint that never disconnects) |
| Naming | `IpcDataHandle` (kernel), `UserDataHandle` (usrlib), `DataObject`, `IpcDataManager`, `kernel/src/ipc/data_manager.rs` |

## 3. Requirements (verifiable)

- **R1 — Message untouched.** `system::ipc::Message` layout, size, and all existing syscall semantics are bit-identical before and after. All existing mailbox/future/connection/binding code paths and their tests pass unchanged.
- **R2 — Two complementary families.** Message syscalls (`20/22/23`) keep their exact behavior; Data syscalls (`24/25/26`) are additive. No existing app changes required.
- **R3 — Data lifecycle is writer→reader→writer.** `write` snapshots bytes and returns a packed `IpcDataHandle`; `read` is legal only for the writer's peer on the object's connection and advances a kernel cursor; `release` is legal only for the writer. Double release and stale-generation tokens are `IpcDataNotFound`.
- **R4 — Authorization matches the current trust model.** Caller identity for write-side checks uses the same rule as `send_message` (`ipc_manager.rs:126-135`): `caller == conn.client_task` distinguishes the client endpoint; the other endpoint is "not the client". `IpcDataRead` requires `caller != DataObject.writer` and caller on that connection, else `IpcDataNotReadable`. Token forged into a message on a *different* connection is rejected by this rule — no payload inspection needed.
- **R5 — Budget accounting.** Per-writer-task outstanding-bytes counter; refunded on `release` and on disconnect GC. `write` rejects over `MAX_DATA_BYTES` (`IpcDataTooLarge`) or over per-task budget (`IpcDataBudgetExceeded`). A hostile client cannot exhaust kernel heap through this surface.
- **R6 — Disconnect GC.** `IpcManager::disconnect` frees every `DataObject` whose `connection` matches and refunds the budgets. No message-drop scanning exists (the kernel never reads payloads).
- **R7 — Error codes.** Four new wire codes in `system/src/error.rs`: `IpcDataTooLarge = 0x0109`, `IpcDataBudgetExceeded = 0x010A`, `IpcDataNotFound = 0x010B`, `IpcDataNotReadable = 0x010C`, with kernel-side `From` projections from a `DataError` enum, following the existing codec. `IpcDataRead` returns a plain byte count on success (never `ERROR_BIT`-set).
- **R8 — Handle pattern compliance.** `IpcDataHandle` is the fifth newtype in `kernel/src/handle.rs` (private fields, `pack`/`from_packed` at the edge only, generation-checked arena). `UserDataHandle` in `usrlib` is an opaque token carrier per R13 of the kernel-owned-handle spec: no decomposition, no re-validation, no layout constants.
- **R9 — usrlib ergonomics.** `Syscall::ipc_data_write(conn, &[u8]) -> Result<UserDataHandle, DataError>`; `Syscall::ipc_data_read(UserDataHandle-like token, &mut [u8]) -> Result<usize, DataError>`; `UserDataHandle::release()` + `Drop` (writer-side). Convenience `usrlib::ipc::send_data(conn, &[u8])` (write + canonical message with the packed token at offset 0) and a matching receive helper, so simple services interoperate without inventing framing.
- **R10 — Treaty discipline.** `system` gains numbers only (syscall numbers, error codes). No Data types enter `system`: payload framing is application protocol (user decision Q4), so there is no boundary layout to publish.
- **R11 — Comprehensive unit tests** (AGENTS.md CRITICAL rule), in the `ipc_manager.rs` style, covering at least: write/read cursor roundtrip; short read then continuation; read past end returns 0; too-large rejection; budget exhaustion then refund-after-release; wrong-reader rejection; release-by-reader rejection; double release; stale-generation token after release+reuse; disconnect GC frees objects and refunds budget; many concurrent objects per connection.
- **R12 — Integration proof in QEMU.** A `bulk_echo` example service under `apps/`: binds `BULKECHO`, on each request message reads the referenced Data to the end, releases nothing (it is the reader), and replies inline with the byte count + a checksum. A client task writes a multi-KB buffer, signals via message, and verifies the echo. Driven via `cargo xtask build` + QEMU run.
- **R13 — All gates green on final tree:** `cargo test -p kernel -- --test-threads=1`, `cargo xtask test`, `cargo xtask build`, x86_32 kernel build unaffected (`cargo build -p rosx-x86` from `arch/x86_32/`).
- **R14 — Documentation updated in the same branch.** `AGENTS.md`: structure tree line `ipc/ (mailbox)` gains the data family (`ipc/ (mailbox, data)` or equivalent), and a one-line statement of the invariant "messages signal, data rides the connection out-of-band; the kernel never interprets message payloads".

## 4. Non-goals / out of scope

- No random-access reads (`offset` arg) — revisit when a real consumer needs them; would require a struct-pointer syscall argument.
- No flow-control *futures* for data budget exhaustion in v1. The writer gets `IpcDataBudgetExceeded` and retries — the message layer already provides the wait primitive (e.g. block on the reader's ack).
- No task-death GC in v1 (fast-follow). Disconnect GC + per-task budget bound the damage meanwhile.
- No stream channels / ring buffers / shared memory — separate future primitive if sustained-stream throughput demands it.
- No MMU isolation work; the design is isolation-*ready*, not isolation.
- No message-boundary changes, no congestion semantics changes.

## 5. Implementation increments

Each increment is small, verified, and commit-worthy on its own. Commit messages carry no prefix.

1. **`system` treaty numbers.** `SyscallNum::{IpcDataWrite=24, IpcDataRead=25, IpcDataRelease=26}` + `TryFrom`; `ErrorCode` additions 0x0109–0x010C + `from_code` + codec tests.
2. **`kernel/src/handle.rs`:** `IpcDataHandle` newtype + roundtrip/domain tests mirroring the existing four.
3. **`kernel/src/ipc/data_manager.rs`:** `DataObject { payload: Vec<u8>, connection: IpcConnectionHandle, writer: TaskHandle, read_cursor: usize }` + `IpcDataManager { arena: GenerationalArena<DataObject, 256>, outstanding: BTreeMap<TaskHandle, usize> }`; constants `MAX_DATA_BYTES = 64 * 1024`, `DATA_BUDGET_BYTES = 256 * 1024`; operations `write / read / release` with R3–R5 rules. Wired into `IpcManager` (owning field); `disconnect` performs the R6 GC. Full unit suite (R11).
4. **`kernel/src/syscall.rs`:** three dispatch arms. `IpcDataWrite` builds a user slice `from_raw_parts` (copy-in, same pattern as `Print`); `IpcDataRead` copies kernel→user into `dst`; success returns packed token / byte count, failure `ErrorCode::to_reg()`.
5. **`usrlib`:** `UserDataHandle` (opaque token + `Drop`), error mirror, syscall wrappers, `send_data` / `receive_data` conveniences (canonical token at payload offset 0).
6. **`apps/bulk_echo`** service + client check (R12); wire into the xtask app list and QEMU flow.
7. **Verification pass + docs (R13/R14).**

## 6. Verification

- Per-increment: `cargo test -p kernel -- --test-threads=1` from increment 2 onward; `cargo build -p usrlib`-equivalent via `cargo xtask apps` from increment 5.
- Final: `cargo xtask test`, `cargo xtask build`, QEMU run of the `bulk_echo` exchange (client writes ≥ 64 KiB across multiple DataObjects, server echoes counts/checksums, budget-exhaustion path exercised by a deliberately oversized write).
- Regression guard: every pre-existing unit and integration test unchanged and green (R1).

## 7. Decision log (resolved 2026-10-15)

| # | Question | Resolution |
|---|---|---|
| 1 | Cursor vs random access | Sequential cursor |
| 2 | Who may release | Writer only; reader has no release (protocol ack instead) |
| 3 | One or many Data per connection direction | Many |
| 4 | DataRef placement / treaty | Application decides the protocol; usrlib provides a canonical convenience only |
| 5 | Limits | Constants 64 KiB / 256 KiB / 256 for now |
| 6 | v1 cleanup scope | Disconnect GC in v1; task-death GC fast-follow |
| 7 | Naming | As proposed: `IpcDataHandle`, `UserDataHandle`, `DataObject`, `IpcDataManager`, `ipc/data_manager.rs` |
