# Plan — Re-cut the IPC syscall ABI from buffer-centric to message-centric (branch `ipc-capability-handle`, HEAD 15bc873)

## 1. Objective

Replace the six-call buffer-centric IPC ABI (IpcBufferAlloc/Write/Read/Dispose + destination-taking IpcSend/IpcSendToClient) with a seven-call message-centric ABI (Create/Write/Send/Read/Receive/Accept/Dispose) in which routing is stamped into the message at create time and the 64-byte cap kills the unbounded-`extend_from_slice` DoS. Keep the storage (`GenerationalArena<_, 256>`), the notification-driven future machinery, both arch stubs, and the integration test `tests-integration/tests/ipc_random.rs` completely untouched.

## 2. Design decisions (resolutions a–e, within the locked decisions)

**a) Syscall numbering — renumber to a contiguous message block 18–24, retire 13/14/16/17.**
New: `IpcCreateMessage = 18`, `IpcWriteMessage = 19`, `IpcSendMessage = 20`, `IpcReadMessage = 21`, `IpcReceiveMessage = 22`, `IpcAcceptMessage = 23`, `IpcDisposeMessage = 24`. Removed variants: `IpcSend`(13), `IpcReceive`(14), `IpcReceiveFromClient`(16), `IpcSendToClient`(17), `IpcBufferAlloc`(18), `IpcBufferWrite`(19), `IpcBufferRead`(20), `IpcBufferDispose`(21). Kept untouched: `IpcConnect = 11`, `IpcDisconnect = 12`, `IpcBind = 15`.
Rationale: the branch is unmerged and every consumer (usrlib, apps, kernel, disk image) rebuilds from source on every build — no external party pins the numbers; the leftover holes 13/14/16/17 are safe because `TryFrom<usize>` returns `Err(())` and the dispatcher already fails closed at `kernel/src/syscall.rs:140` (`Err(_) => 0`). Reusing 18–21 with new meanings is explicitly allowed by "retiring old numbers" and keeps the whole message ABI in one contiguous, documentable block.

**b) `wait_future` yield — keep `FutureResult::IpcMessage(Result<IpcMessage, IpcReceiveError>)`; rename `IpcMessage.buffer_handle` → `IpcMessage.message_handle`; keep `connection_handle` in the result.**
Rationale: the accept path must hand the server the connection to reply on; the connection handle is already the shared bidirectional identifier (one `IpcConnection` holds both `server_mailbox` and `client_mailbox`), so `IpcManager::send_message` constructs the delivered `IpcMessage { message_handle, connection_handle: route.connection }` straight from the stamped route — no reverse map, and `IpcMessageFuture`, `FutureResult`, `ForNotifyingFutures::complete_ipc_message`, and `ports/driven.rs` stay byte-identical.

**c) usrlib shape — `OutgoingMessage` / `IncomingMessage` + the three value helpers as sugar, with `ipc_send_value` / `ipc_receive_value` names preserved and `ipc_send_value_to_client` deleted.**
```rust
pub struct OutgoingMessage { message: IpcMessageHandle, sent: bool }
impl OutgoingMessage {
    pub fn create(connection: IpcConnectionHandle, size: usize) -> Result<Self, IpcSendError>
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), IpcMessageError>
    pub fn send(mut self) -> Result<(), IpcSendError>   // one-shot: consumes self, sets sent, Drop then skips dispose
}
impl Drop // if !sent { let _ = Syscall::ipc_dispose_message(self.message); }  — covers failed send (unsealed, dispose succeeds) and panic-abandonment

pub struct IncomingMessage { message: IpcMessageHandle }
impl IncomingMessage {
    pub fn new(message_handle: IpcMessageHandle) -> Self
    pub fn read(&mut self, dst: &mut [u8]) -> Result<usize, IpcMessageError>
}
impl Drop // dispose — preserves the current ReadableMailbox guarantee that received handles are always disposed

pub fn ipc_send_value(connection: IpcConnectionHandle, value: usize) -> Result<(), IpcSendError>
    // = OutgoingMessage::create(conn, core::mem::size_of::<usize>()) + write(&value.to_ne_bytes()) + send()
pub fn ipc_receive_value(connection: IpcConnectionHandle) -> Result<usize, ReceiveValueError>
    // = Syscall::ipc_receive_message + Syscall::wait_future + IncomingMessage; ReceiveValueError variants/Display unchanged
```
`create` returns `Result<_, IpcSendError>` (see d) so `ipc_send_value`'s `?` needs no new `From`. Deleting `ipc_send_value_to_client` is *justified by LD-3 itself*: direction is now an endpoint property stamped at create, so client→server and server→client are literally the same helper; the server's one call site renames mechanically. No accept-sugar is added — the server keeps its explicit `ipc_accept_message` + `wait_future` + `IncomingMessage::new` structure (LD-9: logic unchanged).

**d) Routing stamping — `MessageRoute` lives inside `MessageBuffer`; destination is resolved at create via caller-task identity; send re-validates the stamped connection; disconnect drains and disposes the client mailbox.**
- `pub(crate) struct MessageRoute { pub(crate) connection: Handle, pub(crate) destination: Handle }` (defined in `kernel/src/ipc/message_buffer.rs`; `destination` is the peer mailbox handle).
- `IpcConnection` gains `client_task: crate::task::TaskHandle`, recorded at `connect(service, caller)`. At `IpcCreateMessage`, the dispatcher passes `kernel().execution_state.current_task()`; the manager stamps `destination = server_mailbox` if `caller == conn.client_task`, else `client_mailbox`. This is the only mechanism consistent with LD-1's "from that connection **and the caller's endpoint**" — there is no per-endpoint handle object today (the server obtains the same `Handle` from a delivered message), and adding one would churn `connect`/`disconnect` and the connection-handle values the integration test prints.
- `send_message(msg_handle)` (destination-free): look up stamped route → **validate `connections.borrow(route.connection)` still succeeds *before* sealing** (preserves the recorded invariant of `send_on_dead_connection_skips_seal`, `ipc_manager.rs:254-268`, and generational slots make stale conns impossible to alias) → seal → `push_back(route.destination, IpcMessage { message_handle, connection_handle: route.connection })`. The caller is *not* re-checked at send — create is the authorization point and the sender owns its own message.
- Disconnect semantics for in-flight stamped messages: `disconnect` first drains the **client** mailbox (`MailboxManager::drain`, new) and disposes each queued `msg.message_handle` in the buffer pool (closes a pre-existing slot leak); the **server** mailbox is shared per binding and is NOT drained — a request still queued there stays readable, and a reply attempt then fails `ConnectionNotFound`. A created-but-unsent message at disconnect stays sender-owned and is freed by `OutgoingMessage::Drop` (direct-syscall-user leak = today's documented BR5 behavior, unchanged).
- `create_message` returns `Result<IpcMessageHandle, IpcSendError>` — `ConnectionNotFound` for dead conn, `InvalidBuffer(IpcMessageError::MessageTooLarge)` / `InvalidBuffer(IpcMessageError::PoolExhausted)` otherwise — so **no new error type is introduced** and `IpcSendError`'s variant names and `Display` strings ("Invalid buffer: Buffer pool exhausted") stay byte-identical, keeping the integration tripwire strings (`ipc_random.rs:11-18`) armed.

**e) Unit-test matrix** — see Steps D and E; full names enumerated there: stale generation on all ops, ABA after slot recycle, too-large at create **and** at write (payload-untouched check), double-send (`Sealed`), write-after-send (`Sealed`), send-on-dead-connection-skips-seal, create-on-dead-connection, accept-before-connect delivers, receive/accept on dead resources yield error futures, disconnect disposes queued client messages, 256-slot wrap (exhaust → dispose → recover, and slot-recycle generation bump), route stamp accessor.

**Error/type renames (applied consistently across system/kernel/usrlib):** `IpcBufferHandle` → `IpcMessageHandle` (alias of `Handle`), `IpcBufferError` → `IpcMessageError` **keeping the existing variant names and Display texts verbatim** (`"Buffer not found"`, `"Buffer is sealed"`, `"Buffer is not sealed"`, `"Buffer pool exhausted"` — the integration forbidden-string check depends on these exact texts) and adding `MessageTooLarge` with Display `"Message too large"`. `pub const MAX_MESSAGE_SIZE: usize = 64;` lives in `system/src/ipc.rs`. `IpcSendError` and `IpcReceiveError` are **unchanged** (only the payload type of `InvalidBuffer`). `From<IpcBufferError> for IpcSendError` (`system/src/ipc.rs:36-40`) becomes `From<IpcMessageError> for IpcSendError` with identical mapping. Inline `[u8; 64]` storage is deferred per LD-7: `MessageBuffer` keeps `Vec<u8>` but `new_with_route` does `Vec::with_capacity(data_size)` (validated `≤ 64`).

**Verified call-site census (grep, this branch):** IPC users are exactly `kernel/src/syscall.rs`, `kernel/src/ipc/{message_buffer,message_buffer_manager,ipc_manager,mailbox_manager}.rs`, `system/src/{ipc,future,syscall_numbers}.rs`, `usrlib/src/{syscall,ipc}.rs`, `apps/shell/src/shell.rs` (l.2, 136, 139 — **needs no edit**), `apps/random_gen_server/src/main.rs` (l.9, 40, 42, 44). `hello_elf`, `dummy`, `snake`, `tetris`, `conway`, `test_suite`, `tests-integration/src/*`: no IPC.

## 3. Steps

### Step A — Syscall numbers (`system/src/syscall_numbers.rs`)
Remove variants `IpcSend`, `IpcReceive`, `IpcReceiveFromClient`, `IpcSendToClient`, `IpcBufferAlloc`, `IpcBufferWrite`, `IpcBufferRead`, `IpcBufferDispose`. Add, in this exact order:
```rust
IpcCreateMessage = 18,
IpcWriteMessage = 19,
IpcSendMessage = 20,
IpcReadMessage = 21,
IpcReceiveMessage = 22,
IpcAcceptMessage = 23,
IpcDisposeMessage = 24,
```
In `TryFrom<usize>`: delete match arms `13/14/16/17/18/19/20/21`, add arms `18..=24` mapping to the new variants; the `_ => Err(())` arm already fails closed. Keep `IpcConnect = 11`, `IpcDisconnect = 12`, `IpcBind = 15` byte-identical.
**Gate:** `cargo test -p system` (green standalone — nothing in `system` references the removed names after this step… `system/src/ipc.rs` does not; verified).
**Why first:** every later step names these variants.

### Step B — Shared types (`system/src/ipc.rs`)
1. Line 6 `pub type IpcBufferHandle = Handle;` → `pub type IpcMessageHandle = Handle;`.
2. Add `pub const MAX_MESSAGE_SIZE: usize = 64;`.
3. `IpcBufferError` → `IpcMessageError` (derives `Debug, Clone, Copy, PartialEq, Eq` unchanged); add variant `MessageTooLarge` + Display arm `write!(f, "Message too large")`; keep the four existing variant names and Display strings verbatim.
4. `impl From<IpcBufferError> for IpcSendError` → `impl From<IpcMessageError> for IpcSendError` (body unchanged); `IpcSendError::InvalidBuffer(IpcBufferError)` → `InvalidBuffer(IpcMessageError)` (variant name `InvalidBuffer` kept deliberately — Display text feeds the integration tripwire).
5. `IpcMessage` (l.78-82): `pub buffer_handle: IpcBufferHandle` → `pub message_handle: IpcMessageHandle`; `connection_handle` and derives unchanged. `IpcMessageFuture`, `Future for IpcMessageFuture`: untouched.
**Gate:** `cargo test -p system`.
**Blast radius note:** this breaks compilation of `kernel` (until Step F), `usrlib` (until Step G), and `random_gen_server` (until Step H). No commit-time green between Steps A–F except `system`/`collections`; this red window is deliberate and contained — proceed straight through.

### Step C — Payload + route (`kernel/src/ipc/message_buffer.rs`)
1. Add `use collections::generational_arena::Handle;` and `use system::ipc::MAX_MESSAGE_SIZE;`:
```rust
#[derive(Clone, Copy)]
pub(crate) struct MessageRoute {
    pub(crate) connection: Handle,
    pub(crate) destination: Handle,
}
```
2. `MessageBuffer` gains field `route: MessageRoute`; `new()` becomes `new_with_route(data_size: usize, route: MessageRoute) -> Self` with `payload: Vec::with_capacity(data_size)`.
3. `write` (currently `self.payload.extend_from_slice(bytes);` unbounded — the DoS fix): inside the `BufferState::WriteOnly` arm, before extending: `if self.payload.len() + bytes.len() > MAX_MESSAGE_SIZE { return Err(IpcMessageError::MessageTooLarge); }`. `ReadOnly → Err(IpcMessageError::Sealed)` unchanged.
4. Add `pub(crate) fn route(&self) -> MessageRoute { self.route }`.
5. `seal`/`read` bodies unchanged except error-type rename.
**Gate:** deferred to Step F (crate-wide red window; verify by inspection that no other symbol changed).

### Step D — Message manager (`kernel/src/ipc/message_buffer_manager.rs`)
New method surface (arena type `GenerationalArena<MessageBuffer, 256>` unchanged per LD-8):
```rust
pub(crate) fn create(&mut self, data_size: usize, route: MessageRoute) -> Result<IpcMessageHandle, IpcMessageError>
    // if data_size > MAX_MESSAGE_SIZE { return Err(IpcMessageError::MessageTooLarge); }  — before add, so no slot is consumed
    // self.buffers.add(MessageBuffer::new_with_route(data_size, route)).map_err(|_| IpcMessageError::PoolExhausted)
pub(crate) fn write(&mut self, handle: IpcMessageHandle, bytes: &[u8]) -> Result<(), IpcMessageError>
pub(crate) fn route_of(&self, handle: IpcMessageHandle) -> Option<MessageRoute>
    // self.buffers.borrow(handle).ok().map(|b| b.route())
pub(crate) fn seal / read / dispose   // unchanged bodies, renamed types
```
**Tests (replace the existing 10, all adapted to `create(data_size, dummy_route)` with `MessageRoute { connection: Handle::new(1, 1), destination: Handle::new(2, 1) }`, plus 5 new — 15 total, keeping the existing names where semantics are unchanged):**
`alloc_write_seal_read_happy_path` → `create_write_seal_read_happy_path`; `cursor_accumulates_across_writes_and_split_reads`; `stale_generation_rejected_on_all_ops`; `aba_stale_handle_fails_after_slot_recycle`; `write_after_seal_returns_sealed`; `read_before_seal_returns_unsealed`; `double_dispose_second_fails`; `seal_twice_second_returns_sealed`; `short_read_then_eof`; `zero_size_write_and_zero_cap_read` → `zero_size_create_and_zero_cap_read` (`create(0, route)`); `pool_exhaustion_then_recover_after_dispose` (256-slot wrap: exhaust at 256, dispose, recover, re-exhaust).
New: `create_rejects_data_size_above_cap` (`create(65, _)` → `MessageTooLarge`, and a subsequent `create(1, _)` succeeds — proves the slot was not consumed); `create_accepts_exactly_cap` (`create(64, _)` ok); `write_beyond_cap_rejected_and_payload_untouched` (write 40 ok; write 40 → `MessageTooLarge`; write 24 ok → exactly 64; write 1 → `MessageTooLarge`; seal; read back exactly the 64 written bytes); `route_of_returns_stamped_route`; `route_of_unknown_handle_returns_none`.

### Step E — Connection manager (`kernel/src/ipc/ipc_manager.rs` + `kernel/src/ipc/mailbox_manager.rs`)
`mailbox_manager.rs`: add
```rust
pub(crate) fn drain(&mut self, handle: MailboxHandle) -> Vec<IpcMessage>   // pop_front to exhaustion; leaves waiter map untouched
```
`ipc_manager.rs`:
1. `use crate::task::TaskHandle;` and `use crate::ipc::message_buffer::MessageRoute;`.
2. `struct IpcConnection { server_mailbox, client_mailbox, client_task: TaskHandle }`.
3. `pub(crate) fn connect(&mut self, service: &str, caller: TaskHandle)` — stores `client_task: caller`; allocation/recycling behavior (and therefore the printed `{index} {generation}` values) unchanged.
4. ```rust
   pub(crate) fn create_message(&mut self, connection: IpcConnectionHandle, caller: TaskHandle, data_size: usize) -> Result<IpcMessageHandle, IpcSendError>
   ```
   Borrow `connections`; `Err` → `Err(IpcSendError::ConnectionNotFound)`; copy out `let destination = if caller == conn.client_task { conn.server_mailbox } else { conn.client_mailbox };` (end borrow), then `self.buffer_manager.create(data_size, MessageRoute { connection, destination }).map_err(IpcSendError::InvalidBuffer)`.
5. Remove `send_to_server` and `send_to_client`; add
   ```rust
   pub(crate) fn send_message(&mut self, message_handle: IpcMessageHandle) -> Result<(), IpcSendError>
   ```
   Order is a hard requirement: `route_of` (absent → `Err(InvalidBuffer(IpcMessageError::BufferNotFound))`) → `connections.borrow(route.connection)` liveness (`Err(IpcSendError::ConnectionNotFound)`, **before any seal**) → `seal` (`map_err(IpcSendError::InvalidBuffer)`) → `mailbox_manager.push_back(route.destination, IpcMessage { message_handle, connection_handle: route.connection })`.
6. Passthroughs `write_buffer/read_buffer/dispose_buffer` → `write_message/read_message/dispose_message` (message-handle typed).
7. `receive_from_all_clients_async` → `accept_message_async(&mut self, binding: IpcBindingHandle) -> FutureHandle`; `receive_from_server_async` → `receive_message_async(&mut self, connection: IpcConnectionHandle) -> FutureHandle`; bodies unchanged.
8. `disconnect`: before removing the connection, `for msg in self.mailbox_manager.drain(connection.client_mailbox) { let _ = self.buffer_manager.dispose(msg.message_handle); }`, then the existing `mailbox_manager.remove` + `connections.remove`.
**Tests (keep the `manager_with_notifier()` / `bind_and_connect()` harness — update `bind_and_connect` to pass `Handle::new(1, 1)` as the client caller; second endpoint = `Handle::new(2, 1)`; 12 tests, replacing the current 4):**
1 `client_send_accept_yields_sealed_message` (adapted `send_to_server_seals_buffer_and_delivers_handle`: create→write→send→`accept_message_async(binding)`→downcast `IpcMessageFuture`→assert `result().unwrap().message_handle == msg_handle` **and** `connection_handle == connection`; `read_message` returns the payload);
2 `server_reply_reaches_client_mailbox` (new: after test-1 flow, `create_message(connection, Handle::new(2, 1), 8)`→send→`receive_message_async(connection)` yields the reply — proves caller-keyed direction);
3 `double_send_of_same_message_is_rejected` (→ `Err(IpcSendError::InvalidBuffer(IpcMessageError::Sealed))`);
4 `send_of_disposed_message_is_rejected` (→ `InvalidBuffer(BufferNotFound)`);
5 `send_on_dead_connection_skips_seal` (adapted verbatim-in-spirit: create as client, `disconnect`, send → `ConnectionNotFound`; `read_message` → `Unsealed`; `dispose_message` ok);
6 `create_message_on_dead_connection_is_rejected` (new);
7 `create_message_rejects_data_size_above_cap` (new: `Err(InvalidBuffer(MessageTooLarge))`);
8 `write_message_enforces_cap` (new: write 64 ok then write 1 → `MessageTooLarge`);
9 `accept_before_connect_delivers_message` (new: `accept_message_async(binding)` on an empty binding mailbox registers a pending waiter; subsequent `connect`+`create_message`+`send_message` completes the same future via `notify_waiters` — pattern copied from `mailbox_manager::test_push_back_notifies_waiters`);
10 `receive_on_dead_connection_yields_error_future` (`ConnectionNotFound`, pattern of existing else-branch);
11 `accept_on_unknown_binding_yields_error_future`;
12 `disconnect_disposes_messages_queued_for_client` (new: server `create_message` as `Handle::new(2,1)` + send; client `disconnect` without reading; then `read_message(reply_handle)` → `Err(BufferNotFound)` — proves the drain+dispose path).
**Gate:** deferred to Step F (kernel compiles as one unit).

### Step F — Kernel dispatcher (`kernel/src/syscall.rs` + `mailbox_manager.rs` test literals) → first kernel gate
`kernel/src/syscall.rs` (keep `#[cfg(not(test))] pub fn handle_syscall(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize` and every non-IPC arm untouched):
- `IpcConnect` arm: `services().ipc_manager.borrow_mut().connect(service, kernel().execution_state.current_task())`.
- Delete arms `IpcSend`, `IpcReceive`, `IpcReceiveFromClient`, `IpcSendToClient`, `IpcBufferAlloc`, `IpcBufferWrite`, `IpcBufferRead`, `IpcBufferDispose`. Add:
```rust
Ok(SyscallNum::IpcCreateMessage) => { let connection = IpcConnectionHandle::unpack(arg1);
    let result = services().ipc_manager.borrow_mut().create_message(connection, kernel().execution_state.current_task(), arg2);
    Box::into_raw(Box::new(result)) as usize }
Ok(SyscallNum::IpcWriteMessage) => { let message_handle = IpcMessageHandle::unpack(arg1);
    let bytes = unsafe { core::slice::from_raw_parts(arg2 as *const u8, arg3) };
    Box::into_raw(Box::new(services().ipc_manager.borrow_mut().write_message(message_handle, bytes))) as usize }
Ok(SyscallNum::IpcSendMessage) => { /* send_message(IpcMessageHandle::unpack(arg1)) boxed */ }
Ok(SyscallNum::IpcReadMessage) => { /* IpcMessageHandle::unpack(arg1); core::slice::from_raw_parts_mut(arg2 as *mut u8, arg3); read_message boxed */ }
Ok(SyscallNum::IpcReceiveMessage) => services().ipc_manager.borrow_mut().receive_message_async(IpcConnectionHandle::unpack(arg1)).pack()
Ok(SyscallNum::IpcAcceptMessage) => services().ipc_manager.borrow_mut().accept_message_async(IpcBindingHandle::unpack(arg1)).pack()
Ok(SyscallNum::IpcDisposeMessage) => { /* dispose_message(IpcMessageHandle::unpack(arg1)) boxed */ }
```
  Import `IpcMessageHandle` on line 9's `use system::ipc::{...}` and drop the now-unused `IpcMessage` import (the struct is built only inside `ipc_manager` now). `IpcDisconnect` arm and its split `arg1/arg2` `IpcConnectionHandle::new` form stay byte-identical. No `arch/x86_64` or `arch/x86_32` file is touched (LD-8): both `syscall_handler(num, a1, a2, a3)` trampolines already forward verbatim.
`kernel/src/ipc/mailbox_manager.rs`: rename the three `IpcMessage` test literals' field `buffer_handle:` → `message_handle:` (lines 132, 155, 191) and the two assertions (lines 136, 163) `.buffer_handle` → `.message_handle`; nothing else.
**Gate (first tranche gate):** `cargo test -p kernel -- --test-threads=1` — green expected. Test count: **186 → 199** (baseline 186, +5 in `message_buffer_manager.rs`, +8 in `ipc_manager.rs`; `mailbox_manager.rs` stays at 3). `--test-threads=1` is mandatory (kernel tests share the global `kernel_services` registry).

### Step G — usrlib ABI + RAII (`usrlib/src/syscall.rs`, `usrlib/src/ipc.rs`)
`usrlib/src/syscall.rs` — delete `ipc_send`, `ipc_send_to_client`, `ipc_alloc_buffer`, `ipc_write_buffer`, `ipc_read_buffer`, `ipc_dispose_buffer`, `ipc_receive`, `ipc_receive_from_client`; keep `ipc_connect`, `ipc_disconnect`, `ipc_bind`, `wait_future` byte-identical. Add exactly:
```rust
pub fn ipc_create_message(connection_handle: IpcConnectionHandle, data_size: usize) -> Result<IpcMessageHandle, IpcSendError>
    // raw_syscall(SyscallNum::IpcCreateMessage as usize, connection_handle.pack(), data_size, 0); debox *mut Result<IpcMessageHandle, IpcSendError>
pub fn ipc_write_message(message_handle: IpcMessageHandle, bytes: &[u8]) -> Result<(), IpcMessageError>
    // raw(19, message_handle.pack(), bytes.as_ptr() as usize, bytes.len())
pub fn ipc_send_message(message_handle: IpcMessageHandle) -> Result<(), IpcSendError>      // raw(20, pack(), 0, 0)
pub fn ipc_read_message(message_handle: IpcMessageHandle, dst: &mut [u8]) -> Result<usize, IpcMessageError>
    // raw(21, pack(), dst.as_mut_ptr() as usize, dst.len())
pub fn ipc_receive_message(connection_handle: IpcConnectionHandle) -> FutureHandle          // raw(22, pack(), 0, 0) → FutureHandle::unpack
pub fn ipc_accept_message(binding_handle: IpcBindingHandle) -> FutureHandle                 // raw(23, pack(), 0, 0) → FutureHandle::unpack
pub fn ipc_dispose_message(message_handle: IpcMessageHandle) -> Result<(), IpcMessageError> // raw(24, pack(), 0, 0)
```
All ≤ 3 payload args on both arches (verified against `usrlib/src/arch/x86_64.rs` rdi/rsi/rdx and `usrlib/src/arch/x86_32.rs` ebx/ecx/edx — both files **unchanged**). Debox via the existing `unsafe { *Box::from_raw(...) }` convention (usrlib/src/syscall.rs:67-121).
`usrlib/src/ipc.rs` — replace `WritableMailbox`/`ReadableMailbox` with `OutgoingMessage`/`IncomingMessage` exactly as in design (c), including both `Drop` impls (unsent-dispose and always-dispose); `ipc_send_value` = `OutgoingMessage::create(connection, core::mem::size_of::<usize>())?` + `write(&value.to_ne_bytes())?` + `send()`; `ipc_receive_value` = `ipc_receive_message` + `wait_future` + `IncomingMessage::new(msg.message_handle)` (`msg.buffer_handle` → `msg.message_handle`), `ReceiveValueError` variants/Display kept with inner type `IpcMessageError`; **delete `ipc_send_value_to_client`**.
**Gate:** `cargo test -p usrlib` (compiles the host build of usrlib; expected green; no test files to add — usrlib is host-untested today and is an ABI shim over the kernel-tested logic; QEMU coverage comes in Step J).

### Step H — App call-site adaptation (`apps/random_gen_server/src/main.rs` only; logic unchanged)
Line 9: `use usrlib::ipc::{ipc_send_value_to_client, ReadableMailbox};` → `use usrlib::ipc::{IncomingMessage, ipc_send_value};`
Line 40: `Syscall::ipc_receive_from_client(binding)` → `Syscall::ipc_accept_message(binding)`
Line 42: `ReadableMailbox::new(msg.buffer_handle)` → `IncomingMessage::new(msg.message_handle)` (binding name `_request` and scope-end dispose preserved — the pool-arithmetic BR6 invariant: without it the 256-slot pool exhausts at ~iteration 257 and prints the tripwire `Buffer pool exhausted`)
Line 44: `ipc_send_value_to_client(msg.connection_handle, value)` → `ipc_send_value(msg.connection_handle, value)`
`apps/shell/src/shell.rs`: **zero edits** — line 2 `use usrlib::ipc::{ipc_receive_value, ipc_send_value};`, the `println!("RANDOM Server: {} {}", ipc_connection.index, ipc_connection.generation);` print, the `for i in 1..260` loop, and all output strings stay byte-identical by construction.
**Gate:** `cargo xtask apps` (builds `random_gen_server` + all USER_APPS for `arch/x86_64/rosx-user.json`).

### Step I — Workspace + cross-arch build tranche
Run the three build/unit gates in Section 4 items 4–7. No file changes; fix fallout if any (expected: none).

### Step J — Integration + warning gates
Run Section 4 items 8–9 against `tests-integration/tests/ipc_random.rs` **unmodified**. Verify the wrap assertion survives: `ipc_random.rs:123-130` reads generations only from `RANDOM Server:` lines (connection handles; connections arena semantics unchanged), and the 259 value round-trips exercise the message pool at a steady state of ≈2 live slots (request disposed by server-scope `IncomingMessage` drop, reply disposed by client `ipc_receive_value` drop) — 259×2 allocations never exhaust 256 slots while both Drop guarantees hold.

## 4. Acceptance criteria (exact commands; run order shown)

Locally runnable (fast unless noted); 8 is long-running (QEMU).

1. `cargo test -p system` — all green (after Step B).
2. `cargo test -p kernel -- --test-threads=1` — all green; **199 kernel unit tests** expected (186 baseline + 13 added in Steps D/E); after Step F.
3. `cargo test -p usrlib` — all green; after Step G.
4. `cargo xtask apps` — green (cold build-std: minutes); after Step H.
5. `cargo xtask test --skip-integration` — green; this is the canonical CI unit command (apps build + `cargo test --workspace --exclude tests-integration -- --test-threads=1`): kernel 199, `collections` 15 (untouched), shell 5 (untouched).
6. `cargo xtask build` — green (kernel + disk image, no QEMU launch; CI build-job equivalent).
7. `cd arch/x86_32 && cargo build -p rosx-x86` — green. x86_32 parity gate: compiles `system` + `kernel` + `usrlib` + `shell` for `rosx-i686` via that dir's `.cargo/config.toml`; catches any 3-arg-budget violation or rename drift. (Local only — CI has no x86_32 job; needs `rustup component add rust-src`.)
8. `cargo test -p tests-integration --test ipc_random` — **long-running** (~21 s boot + kernel build inside `kernel_build.rs`; needs `qemu-system-x86_64`). Must pass with the test file **unmodified**: 259× `RANDOM Server: `, 259× `RANDOM Value requested`, 259× `RANDOM Value: `, one connection generation above first, consecutive values differ, none of the six failure strings. Gate is the stem (`--test ipc_random`), never a positional test-name filter.
9. Warning fingerprint (preservation gate, not absolute zero):
   - `cargo clean -p tests-integration -p tempfile && cargo test -p tests-integration --no-run 2>&1 | grep -c 'warning.*deprecated'` — expected output **`1`** (exactly the pre-existing tempfile deprecation).
   - `cargo test --workspace --exclude tests-integration --no-run 2>&1 | grep -E '^warning:'` — expected: no warning lines beyond the recorded baseline for the unit tranche (zero new warning categories introduced by Steps A–H).

## 5. Risks & mitigations

1. **Breaking the 259× integration assertion (pool exhaustion).** Steady-state message-pool occupancy is exactly 2 slots only if *both* Drop guarantees hold: server wraps every accepted handle in `IncomingMessage` (`main.rs` line 42 — do not rename the `_request` binding or narrow its scope), and `ipc_receive_value` drops its `IncomingMessage` on every path including `ShortRead`. A regression surfaces early: QEMU prints `Buffer pool exhausted` (tripwire string) at ~iteration 257, and unit tests 1/2/12 in Step E pin the dispose paths.
2. **Handle-pool wrap semantics.** `GenerationalArena<MessageBuffer, 256>` is untouched (LD-8); the integration wrap check observes *connection* generations (`ipc_random.rs:123-130`), and `connect`/`disconnect` keep identical slot recycling (`client_task` is payload, not slot-allocation logic). If the assert `pairs.iter().any(|g| g.1 > first_generation)` ever goes red, the bug is in `connect`, not in the message layer — bisect there first.
3. **Seal-before-liveness-check inversion.** `send_message` must check `connections.borrow(route.connection)` *before* sealing (test 5 mirrors the recorded `send_on_dead_connection_skips_seal`); sealing first would strand messages as `ReadOnly` with no receiver and change observable error paths.
4. **`current_task()` unwrap in the `IpcCreateMessage`/`IpcConnect` arms.** `kernel().execution_state.current_task()` (`state.rs:70-72`) unwraps `Option`. Safe for all real IPC callers — shell and random_gen_server are scheduled tasks (`arch/x86_64/src/main.rs:75-76`), and today's blocking `wait_future` already unwraps the same field (`kernel.rs:226`) on every empty-mailbox receive. Risk only for hypothetical new IPC syscalls issued from the bare scheduler/main-thread context before the first `switch_to_task` — do not add such callers.
5. **Client-endpoint identity ABA.** `client_task` is a `TaskHandle` slot; if the connecting task terminates and its slot is recycled, a *different* task holding a leaked connection handle is classified as the client endpoint (routes to the server mailbox). RosX has no per-task handle ACLs today (any handle-holder could already call `IpcSendToClient`), so this is no regression — document in the commit message; the principled fix (tear down connections on task termination) is out of scope (LD-9).
6. **x86_32 parity.** `usrlib/src/arch/x86_32.rs` is shape-locked to `raw_syscall(num, a1, a2, a3)` via `int 0x80` (eax/ebx/ecx/edx) and needs **no change** — verified that every new call fits 3 payload args (Section Step G). `usize` = 4 bytes there: `size_of::<usize>() = 4 ≤ 64`, `HalfSize` shrinks per `generational_arena.rs` cfg and `pack/unpack` adapt. Command 7 is the only real gate; CI will not catch x86_32 drift, so the coder must not skip it.
7. **Red compile window Steps A→F.** The system-crate renames break kernel/usrlib builds until Step F/G; intermediate commits inside the tranche will not build. Mitigation: keep Steps A–F as one reviewable tranche, gate strictly at Step F (`cargo test -p kernel -- --test-threads=1`), and do not insert any Step-H app change before F (the `cargo xtask apps` gate at Step H would otherwise fail for unrelated reasons).
8. **Stale mixed ABI artifacts.** A kernel rebuilt without rebuilding apps sends numbers 13/14/16/17, which now `TryFrom` → `Err` → dispatcher returns `0`. Harmless-but-confusing during development; mitigated by always running acceptance command 5/6 (`xtask` rebuilds apps first) before QEMU sessions.
9. **Display-string/tripwire coupling.** Variant names `BufferNotFound`/`PoolExhausted` and their Display texts ("Buffer not found", "Buffer pool exhausted") must NOT be "cleaned up" into message-centric wording — the integration forbidden-string tripwires (`ipc_random.rs:16-17`) match those exact texts, and renaming them would silently disarm real-failure detection while the count assertions still pass in the happy path.
10. **`disconnect` drain ordering.** Drain the client mailbox and dispose its queued handles *before* `mailbox_manager.remove` + `connections.remove`; draining the *server* mailbox would drop other connections' in-flight requests (it is shared per binding) — test 12 pins the client-mailbox behavior; nothing in the repo depends on reads-after-disconnect succeeding.
