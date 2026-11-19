# Input

## Invariants

- Input policy lives in the user-space `inputd` daemon and device semantics in driver daemons (`ps2d`); the kernel delivers opaque event frames only.
- The `ps2d` decoder decodes Ctrl+C as `'\x03'`; `inputd` routes it to its single registered watcher as `CONTROL` and never to readers or the char buffer, and without a watcher it is an ordinary char.
- `UNWATCH` always replies `UNWATCHED`: a pending receive can only be released by a completing message (there is no FutureDrop syscall).

## Design

Input architecture — policy lives in the user-space `INPUTD` daemon (`inputd`), device semantics in driver daemons (`ps2d`); the kernel delivers opaque event frames only. The `ps2d` decoder tracks left `Ctrl` (scancode `0x1D`/`0x9D`) beside shift and decodes Ctrl+C as `'\x03'` (Ctrl+anything else stays the plain char). `inputd`'s mux holds one optional **watcher** connection: `WATCH` (wire tag 6) registers the sender, `UNWATCH` (7) evicts it and always replies `UNWATCHED` (9) — the ack exists because a pending `IpcMessageFuture` can only be released by a message completing it (there is no FutureDrop syscall), so the watcher needs one last frame to drain its receive before disconnecting. While a watcher is registered, an incoming `'\x03'` is routed to the watcher as `CONTROL` (8) and reaches neither readers nor the char buffer; without a watcher it is an ordinary char on the normal path. A dead watcher is evicted by `forget` when its reply send fails. The shell registers a watcher only around foreground launches (waitset over {child completion future, control receive}): a `CONTROL` frame echoes `^C` and calls the `Kill` syscall on the child's completion-future handle; at the bare prompt `'\x03'` arrives as a plain char and the prompt loop echoes `^C`, clears its line buffer and re-prompts.
