# IPC Design

## What it is

Synchronous, rendezvous-based message passing. One client sends a message to a named endpoint and blocks until the server replies. One server waits on an endpoint and processes one request at a time. There is no queuing: an endpoint holds at most one in-flight exchange.

---

## Key files

| File | Role |
|---|---|
| `system/src/ipc.rs` | `Message` type, endpoint ID constants, protocol tag constants |
| `kernel/src/ipc.rs` | `EndpointRegistry`, blocking helpers, `IpcClientFuture`, `IpcServerFuture` |
| `kernel/src/syscall.rs` | Syscall handlers that expose IPC to userland tasks |
| `usrlib/src/syscall.rs` | Userland wrappers around raw syscalls |
| `kernel/src/random_service.rs` | Reference kernel-side server implementation |

---

## Message format

```
Message {
    tag:         u64   // identifies the operation (protocol-defined)
    words:       [u64; 4]  // inline arguments or return values
    payload_ptr: usize // pointer to caller-owned buffer (if any)
    payload_len: usize
}
```

`tag` doubles as a discriminant for request type and reply type — the protocol defines what each value means. Words carry small scalar values. `payload_ptr`/`payload_len` are for larger data; the kernel does not copy the buffer, so the pointer must remain valid for the duration of the call.

---

## Endpoint state machine

Each endpoint is identified by a `u32` ID and transitions through four states:

```
         recv() on idle            send() when server waiting
Idle ──────────────────> WaitingReceiver ──────────────────> WaitingReply
 ^                                                                  |
 |   reply()                                                        |
 └──────────────────────────────────────────────────────────────────┘

         send() when idle
Idle ──────────────────> WaitingCaller ──────────────────> WaitingReply
                                           recv() arrives
```

| State | Meaning |
|---|---|
| `Idle` | No pending activity |
| `WaitingReceiver(server)` | Server called `recv()` before a client arrived |
| `WaitingCaller(client, msg)` | Client called `send()` before the server was ready |
| `WaitingReply(client)` | Server has the message and is computing a reply |

A second `send()` or `recv()` while an exchange is in progress returns `IpcError::EndpointBusy`. This means endpoints are single-request: no fan-out, no queuing.

---

## Blocking and wakeup

The kernel's scheduler cooperates with IPC through two `Future` types in `kernel/src/ipc.rs`:

- **`IpcClientFuture(task)`** — resolves when `has_client_reply(task)` is true
- **`IpcServerFuture(task)`** — resolves when `has_server_delivery(task)` is true

When a task must block, the caller registers the appropriate future with the `future_registry`, then calls `kernel().wait_future(handle)`. The scheduler polls registered futures each tick and wakes the task when its future resolves.

Completed deliveries/replies are held in two side-tables on `EndpointRegistry`:

- `server_deliveries: Vec<(TaskHandle, ReplyToken, Message)>`
- `client_replies: Vec<(TaskHandle, Message)>`

The woken task calls `take_server_delivery` or `take_client_reply` to retrieve its data.

---

## Kernel-side server pattern

The helper `kernel_ipc_recv_blocking(id) -> (ReplyToken, Message)` in `kernel/src/ipc.rs` encapsulates the blocking logic for kernel tasks. A kernel server looks like:

```
create endpoint
loop:
    (token, msg) = kernel_ipc_recv_blocking(endpoint_id)
    reply = handle(msg)
    registry.reply(token, reply)
```

`random_service.rs` is the canonical example. New kernel services should follow the same shape.

---

## Userland call flow

### Client (send + blocking wait for reply)

Syscall `IpcSend(endpoint_id, &request, &mut reply)`:

1. `registry.send(id, current_task, msg)`
   - If `Idle` → store `WaitingCaller`, return immediately (no server yet)
   - If `WaitingReceiver` → push to `server_deliveries`, transition to `WaitingReply`
2. Register `IpcClientFuture`, call `kernel().wait_future()`
3. On wakeup: `take_client_reply(current_task)` → write to `reply_ptr`

The client always blocks after `send()` regardless of which branch is taken.

### Server (recv + reply via token)

Syscall `IpcRecv(endpoint_id, &mut msg, &mut token)`:

1. `registry.recv(id, current_task)`
   - `ServerHasMessage` → deliver immediately (client was waiting)
   - `ServerBlocked` → register `IpcServerFuture`, block, take delivery on wakeup
2. Return `(token_u64, msg)` to userland

Syscall `IpcReply(token_u64, &reply)`:

1. Decode token from packed u64 (see below)
2. `registry.reply(token, reply)` → push to `client_replies`, endpoint returns to `Idle`

### Token wire encoding

`ReplyToken` wraps a `TaskHandle { index: u8, generation: u8 }`. The kernel encodes it as:

```
token_u64 = index | (generation << 8)
```

Decoded on `IpcReply`:

```
index      = token_u64 & 0xFF
generation = (token_u64 >> 8) & 0xFF
```

---

## Endpoint IDs

Defined in `system/src/ipc.rs`:

| Constant | ID | Owner |
|---|---|---|
| `endpoint::TERMINAL` | 1 | (not yet implemented) |
| `endpoint::KEYBOARD` | 2 | (not yet implemented) |
| `endpoint::RANDOM` | 3 | `random_server()` |

Add new IDs here. There is no dynamic registration from userland; `IpcEndpointCreate` is available as a syscall but kernel servers create their endpoints at startup.

---

## Protocol tags

Each service defines its own tag constants under `system/src/ipc.rs`. Current example:

```
random::TAG_NEXT  = 1  // request: generate next value
random::TAG_VALUE = 2  // reply: words[0] holds the u64 value
```

Add new protocols as `pub mod <service>` blocks in `system/src/ipc.rs` following the same pattern.

---

## How to add a new kernel service

1. Add an endpoint ID constant to `system/src/ipc.rs` under `pub mod endpoint`
2. Define request/reply tag constants as a `pub mod <service>` block in the same file
3. Create `kernel/src/<service>.rs` with a server function that follows the `random_server` pattern
4. Register the server as a task during kernel startup
5. Expose a convenience method on `usrlib/src/syscall.rs::Syscall` (like `random_next_u64`) for userland callers

---

## Known limitations

- **Single in-flight request per endpoint.** A second client calling `send()` while an exchange is active gets `EndpointBusy`. There is no queue; the caller must retry.
- **No timeout on IPC.** A server that stalls blocks the client forever.
- **Token is opaque u64 passed to userland.** A userland server could forge a token. This is acceptable for a trusted-server model but not for an untrusted multi-process design.
- **`payload_ptr` is not validated.** The kernel uses the raw pointer as-is; passing a bad pointer is undefined behaviour.
- **`IpcRecv` syscall not using `kernel_ipc_recv_blocking`.** The syscall handler in `syscall.rs` duplicates the blocking logic that the helper encapsulates. If the helper changes, `IpcRecv` must be updated in parallel. This is a known inconsistency to resolve.
