# Keyboard over INPUTD — user-space input architecture and direct scancode delivery — Implementation Plan

> Planned 2026-10-25 at `origin/main` `2f03e2f` (post-#77: `kernel::interrupt` dispatch core, `PicController`, `LocalApic` are in the baseline).
> Inputs: the orchestrator conversation of 2026-10-24/25 (INPUTD topology by the user), the #77 diff, and per-anchor verification against the tree
> (worktree tree byte-identical to `origin/main` — `git diff origin/main HEAD` empty — so every quoted line below applies to `main`).
> This plan writes documents only; the Coder implements. Repo law applies: `kernel/` is `no_std`, deps limited to `system` + `lazy_static`,
> **no comments ever** (assembly excepted), unit tests mandatory, `cargo build --workspace` is forbidden, commit subjects are bare imperative
> (no prefix). POSIX `grep -E` gates only, never `rg`. A gate marked `unverified` is not binding until the orchestrator's baseline pass proves it.

---

## 0. Locked decisions (user-approved — CLOSED, not open questions)

- **LD1 — Topology (user, verbatim intent 2026-10-25):** one server service `INPUTD`. Every app that needs input connects to it and uses plain
  request/response (`READ` then wait for the response). The keyboard driver is **also a client** of `INPUTD`, but its message is a **push** of a
  keystroke. `INPUTD` owns data flow and fan-out and has **zero knowledge of low-level devices**; the driver owns device semantics.
- **LD2 — INPUTD fan-out policy:** one char is delivered to exactly one reader (FIFO handoff to the longest-waiting `READ`), never broadcast.
  Keystrokes arriving with no pending reader are buffered in a FIFO bounded to 32 (mirrors `MAILBOX_DEPTH`); on overflow the **oldest** buffered
  char is dropped. `READ_NOWAIT` answers `KEY` from the buffer if non-empty, else `NO_KEY`.
- **LD3 — Wire payload is a char stream now** (exact parity with today's `read_char`). No press/release key-event shape; revisit only when an app
  needs keys the current decoder cannot produce (e.g. arrows). Consequence: the Phase-B shift fix changes no wire bytes.
- **LD4 — The deferred shift-state fix** (`docs/todo.md:32`, decided 2026-10-20) **lands in Phase B, inside the ps2d decoder** — a pure, host-unit-tested
  state machine in user space. CapsLock stays out of scope. ROM app renames are a separate follow-up (recorded in §8).
- **LD5 — x86_32 boots `inputd` and `ps2d`.** `usrlib` is shared by both arch ports, so the re-skin of `read_char` strands x86_32 unless the i686
  roster carries both daemons; they therefore land in the same cutover commit (C4).
- **LD6 — Phase-B hardware channel:** `ps2d` binds service `PS2KEYBOARD`. The kernel delivers a raw scancode frame into that binding's mailbox via a
  new **kernel-internal** method `deliver_to_service` (no new syscall). Provenance = the envelope `conn` word is stamped
  `KERNEL_EVENT_CONN: u32 = 0x8000_0000`; see §2.3 for the unforgeability proof. The service name and frame codec live in the `input-protocol` crate,
  referenced from the arch ports; `kernel/` itself names neither service nor scancode.
- **LD7 — `ReadChar` (4) and `TryReadChar` (9) are deleted** from the `SyscallNum` treaty; the number hole is the reservation (never re-mapped),
  enforced by a treaty test (§5 C7).
- **LD8 — Waitset (#76) is out of scope.** Nothing in this plan depends on it; it is pure ergonomics for later app-side multiplexing.
- **LD9 — Atomic-cutover rule (architect boundary check):** the usrlib re-skin and both boot rosters ship together in **C4**; the arch handler swap and
  the ps2d v2 decoder ship together in **C6**. Splitting either pair leaves an intermediate boot whose input chain is severed and would knowingly break
  `boot_and_shell`.
- **LD10 — Zero `ErrorCode` variants are added.** The four `ALL_ERROR_CODES`-class inventory sites
  (`system/src/error.rs:53`, `kernel/src/handle.rs` test mod, `usrlib/src/error.rs` test mod, inline list in `system/src/ipc.rs`
  `packed_handle_space_is_disjoint_from_error_space`) are **untouched** by this plan.
- **LD11 — `input-protocol` is a wire-only crate** (deps: `system` only, like the `wire.rs` half of `vfs-protocol`). No client module: usrlib depends on
  it, so a client module using usrlib would form a dependency cycle. The connect-retry loop is written twice (inputd main, ps2d main) and once inside
  usrlib, following `vfs-protocol/src/client.rs` values verbatim.
- **LD12 — No integration test for the PS2KEYBOARD internal channel** — it is exercised end-to-end by the existing scripted QEMU tests, which drive
  real keystrokes through the whole chain at C4 and again at C6 (see §7).

---

## 1. Verified baseline facts (every quoted string re-verified against `origin/main` `2f03e2f`)

- **F1 — Keyboard IRQ path on x86_64 after #77** (`arch/x86_64/src/interrupts.rs`): registration
  `    let _ = interrupt::register_handler(InterruptIndex::Keyboard.as_irq(), keyboard_handler);` (`:68`);
  handler `:111-115`:
  `fn keyboard_handler(_irq: u32) {` / `    let mut port: Port<u8> = Port::new(KEYBOARD_PORT);` / `    let scancode: u8 = unsafe { port.read() };` /
  `    kernel().enqueue(HardwareInterrupt::Keyboard { scancode });` / `}`; IDT stub `:117-119` (`interrupt::dispatch_irq(InterruptIndex::Keyboard.as_irq())`);
  `const KEYBOARD_PORT: u16 = 0x60;` (`:104`); import `use kernel::messages::HardwareInterrupt;` (`:8`); `enable_keyboard()` (`:71-77`) flushes 0x60 then
  `interrupt::unmask`.
- **F2 — Keyboard IRQ path on x86_32** (`arch/x86_32/src/interrupts.rs:165-169`):
  `extern "x86-interrupt" fn keyboard_interrupt_handler(_frame: InterruptStackFrame) {` / `    let scancode = unsafe { inb(0x60) };` /
  `    unsafe { outb(PIC_MASTER_CMD, PIC_EOI) };` / `    kernel().enqueue(HardwareInterrupt::Keyboard { scancode });` / `}`.
  `enable_keyboard()` at `:139-146`; IDT entry `:87`; `enable_keyboard()` is called from `arch/x86_32/src/cpu.rs:17`. x86_32 does **not** use the
  dispatch core; `deliver_to_service` (§3.1) is orthogonal to it, so x86_32 needs no interrupt-scheme migration.
- **F3 — Legacy in-kernel pipeline (the deletion inventory, Appendix A):** `kernel/src/messages.rs` (whole, 6 lines);
  `kernel/src/keyboard.rs` (whole, 349 lines — buffer `:11-13`, single waiter slot `KEYBOARD_WAITER_HANDLE` `:15-19`, `push_key` `:21-29`,
  `pop_key` `:31-33`, `KeyboardFuture` `:35-59`, `Key` + `from_scancode_set1` `:61-242`, `KeyboardEvent::from_key` `:243-338`);
  `kernel/src/kernel.rs` `KernelInterruptHandler` `:69-87`, `KERNEL_INTERRUPT_HANDLER` `:70`, `Kernel::enqueue` `:203-208`;
  `kernel/src/ports/driven.rs` `ForHandlingHardwareInterrupts` `:43-45`; `kernel/src/lib.rs` `mod keyboard;` (`:20`) and `pub mod messages;` (`:22`).
- **F4 — Scheduler sites** (`kernel/src/scheduler/scheduler.rs`): `NoopInterruptHandler` + `NOOP_INTERRUPT_HANDLER` `:35-38`;
  `SchedulerPorts` field `interrupt_handler: &'static dyn ForHandlingHardwareInterrupts,` (`:62`) with matching `new()` parameter (`:72`) and `noop()`
  (`:87`); struct fields `hw_interrupt_queue: VecDeque<HardwareInterrupt>,` (`:99`) and `interrupt_drain_buf: Vec<HardwareInterrupt>,` (`:102`);
  `step()` drain preamble `:117-120` (`core::mem::take` + `drain_hardware_interrupts` + handle-loop) and postamble `:129-130`
  (`interrupts.clear();` / `self.interrupt_drain_buf = interrupts;`); `push_hardware_interrupt` `:150-152`; `drain_hardware_interrupts` `:209-213`.
  Tests to delete: `push_hardware_interrupt_adds_to_queue` (`:441`), `step_handles_interrupts_before_timer_expiry` (`:866`),
  `step_routes_hardware_interrupts` (`:975`), and the `ForHandlingHardwareInterrupts` test impls (`RecordingInterruptHandler` `:797`,
  `StepInterruptHandler` `:816`). `kernel/src/scheduler/mod.rs` `kernel_ports()` (`:19-31`) passes
  `        &crate::kernel::KERNEL_INTERRUPT_HANDLER,` (`:28`) — one argument dropped at C7. The `SchedulerFactory` type (`mod.rs:17`) is
  `fn(&'static dyn ForSwitchingTaskContext, &'static dyn ForCompletingExpiredTimers) -> Box<Scheduler>` — **unchanged throughout**; `KConfig` and both
  arch `main.rs` factory wirings are never touched.
- **F5 — Legacy syscall surface:** arms `Ok(SyscallNum::ReadChar) => {` `kernel/src/syscall.rs:38-50` (pop-or-register-future-or-wait) and
  `Ok(SyscallNum::TryReadChar) => {` `:77-79` (`crate::keyboard::pop_key().map_or(0, |c| c as usize)`);
  treaty `    ReadChar = 4,` (`system/src/syscall_numbers.rs:8`), `    TryReadChar = 9,` (`:13`), arms `            4 => Ok(Self::ReadChar),` (`:36`),
  `            9 => Ok(Self::TryReadChar),` (`:41`).
- **F6 — usrlib:** `pub fn read_char() -> char` (`usrlib/src/syscall.rs:82-85`) and `pub fn try_read_char() -> Option<char>` (`:87-95`) are the only
  consumers of `SyscallNum::ReadChar`/`TryReadChar` in usrlib. `raw_syscall` is `pub` on both arches (`usrlib/src/arch/x86_64.rs:1`,
  `usrlib/src/arch/x86_32.rs:1`): `pub fn raw_syscall(num: usize, arg1: usize, arg2: usize, arg3: usize) -> usize`. Wrappers available to daemons:
  `wait_for_message` (`:52`), `ipc_connect` (`:104`), `ipc_bind` (`:122`), `ipc_send` (`:131`, envelope built via `Message::new(connection.packed(), data)`),
  `ipc_receive_message` (`:149`), `ipc_accept_message` (`:161`). `ReceivedMessage::connection()` returns `Option<UserConnectionHandle>` and
  `ReceivedMessage::data()` returns `&[u8; MESSAGE_PAYLOAD_BYTES]` (`usrlib/src/ipc.rs:19-24`); `UserConnectionHandle::from_token` is `pub(crate)`
  (handle.rs `:44`, `:48`) so `connection()` is the only public handle-recovery path.
- **F7 — App read sites (blast radius):** `apps/shell/src/shell.rs:38` `let c = Syscall::read_char();`, `apps/snake/src/main.rs:143,206`,
  `apps/tetris/src/main.rs:216,262`, `apps/conway/src/main.rs:126`. All go through the two usrlib wrappers; **no app edits are needed** — the re-skin
  migrates them atomically.
- **F8 — IPC routing and mailbox facts:** `IpcManager::send_message` routes by caller identity (`kernel/src/ipc/ipc_manager.rs:126-142`:
  `if caller == conn.client_task { conn.server_mailbox } else { conn.client_mailbox }`) — a server's reply is therefore already a push into the
  client mailbox. `accept_message_async` pops the binding's server mailbox (`:144-153`); `connect` (`:93-114`) stores `client_task`; the registry is
  `    registry: BTreeMap<String, IpcBindingHandle>,` (`:51`); `IpcServerBinding` carries the mailbox handle (`:26-39`, created `:85`).
  `MailboxManager::push_back` (`mailbox_manager.rs:58-70`) calls `notify_waiters` (`:93-109`), which does `complete_ipc_message` then `notify` →
  `FutureRegistry::notify` (`kernel/src/future.rs:160-163`) → `wake_controller.wake_tasks` → `scheduler.borrow_mut().wake_tasks`
  (`kernel_services.rs` `SchedulerWakerUseCase`). `MAILBOX_DEPTH: usize = 32` (`kernel/src/ipc/mailbox.rs:4`). `push_back` / `pop_front_async` /
  waiter-complete are allocation-free on the wake path.
- **F9 — Conn-word token space (sentinel facts):** `pub const ERROR_BIT: usize = 1 << 31;` (`system/src/error.rs:1`),
  `pub const fn is_token(raw: usize) -> bool { raw < ERROR_BIT }` (`:3-4`). `Handle::pack` masks generation with
  `GENERATION_MASK = (ERROR_BIT - 1) >> INDEX_BITS` and `INDEX_BITS = 16` (`kernel/src/handle.rs:3-5,22-28`) — **every real packed handle is
  `< 2^31`, i.e. a token**. `from_packed` requires `is_token` (`handle.rs:56-62` and twins); `send_message` rejects a non-token conn word — existing test
  `send_on_error_bit_conn_word_rejected` (`ipc_manager.rs`) proves the rejection with an `ERROR_BIT`-bearing conn. `Message` is
  `#[repr(C)] { conn: u32, data: [u8; 60] }` with `EMPTY` conn `0` and `Message::new(conn, data)` (`system/src/ipc.rs:1-18`);
  `MESSAGE_PAYLOAD_BYTES: usize = 60` (`:2`).
- **F10 — IRQ-context precedent:** the timer handler (`arch/x86_64/src/interrupts.rs:106-109`) calls `kernel().preempt()`, which can
  `switch_to_scheduler()` from inside `dispatch_irq`; the keyboard handler today calls `kernel().enqueue()` from the same context. `deliver_to_service`
  (push_back → complete → wake) is strictly the same class of work as the existing F8 wake chain.
- **F11 — Toolchain/CI commands:** `cargo xtask test --skip-integration` (`.github/workflows/ci.yml:31`), `cargo xtask build` (`:51`).
  xtask `USER_APPS` verbatim at `xtask/src/main.rs:5`:
  `const USER_APPS: [&str; 12] = ["pi", "randomd", "echo", "bulk", "pong", "ping", "vfsd", "vfs", "snake", "tetris", "conway", "threads"];`
  — `build_apps()` and the ROM packer consume the same list.
- **F12 — Workspace shape:** `Cargo.toml` members `"vfs-protocol",` (`:6`), `"apps/test_suite",` (`:22`), `"apps/threads",` (`:23`), `"xtask",` (`:24`);
  default-members include `"vfs-protocol",` (`:31`). `vfs-protocol` lives in default-members as a host-testable lib → `input-protocol` mirrors that.
  New app crates join `members` only (like `pi`, `randomd`, …), **not** default-members.
- **F13 — App crate shapes:** binary-only precedent `apps/ping/Cargo.toml` (`[[bin]] name/bench=false/test=false`, deps `system`, `usrlib`,
  `[target.'cfg(target_os = "none")'.dependencies] usrlib features=["runtime"]`); testable-daemon precedent `apps/vfsd/Cargo.toml`
  (`[lib] name/path` + `[[bin]] test=false`). Main heads are `#![no_std]` + `extern crate alloc;` (`apps/ping/src/main.rs:1-3`).
  A bin with `test = false` is not compiled by host `cargo test`; it is compiled by `cargo xtask build` once it is in `USER_APPS` — hence each app commit
  also adds its `USER_APPS` entry (compile + ROM proof), while **booting** waits for the C4 rosters.
- **F14 — Boot rosters:** x86_64 `arch/x86_64/src/main.rs` embeds ELFs and schedules `RANDOMD, ECHO, BULK, PONG, VFSD, SHELL` (`:79-90`), insertion point
  for the daemons is between `    let _ = kernel.schedule(new_elf_task(VFSD_ELF));` (`:88`) and
  `    static SHELL_ELF: &[u8] = include_bytes!("../../../target/rosx-user/release/shell");` (`:89`).
  x86_32 `arch/x86_32/src/main.rs:56-57`:
  `    static SHELL_ELF: &[u8] = include_bytes!("../../../target/rosx-i686-user/release/shell");` /
  `    let _ = kernel.schedule(kernel::task::new_elf_task(SHELL_ELF));` — daemons insert **above** with the same `../../../target/rosx-i686-user/release/`
  path pattern. The i686-user ELF build recipe for apps is doc-only in AGENTS.md ("build it for `rosx-i686-user` first … with the explicit `-Z` build-std
  flags xtask uses for x86_64"); no CI job exists for x86_32.
- **F15 — Daemon/server precedents quoted:** reply loop `apps/vfsd/src/main.rs:86-95` (`received.connection()` → `dispatch` →
  `let _ = Syscall::ipc_send(connection, &reply.encode());`); retry-connect `vfs-protocol/src/client.rs:12-13,61-67`
  (`const CONNECT_ATTEMPTS: usize = 50;`, `const CONNECT_RETRY_DELAY_MS: u64 = 100;`); service-name constant
  `pub const SERVICE_NAME: &str = "VFS";` (`vfs-protocol/src/wire.rs:4`).
- **F16 — QEMU harness:** `tests-integration/tests/` contains `boot_and_shell.rs` (test `shell_echoes_keystrokes` `:19`, drives
  `session.send_text("hello")` `:22`, `send_line("ls")` `:31`, `send_line("foo")` `:36`) and `shell_command_arguments.rs`; keystrokes go through
  `qemu.rs:160-165 send_key` → monitor `sendkey`. **The harness has no `-cpu` knob**, so the no-APIC PIC-fallback gate cannot be scripted without
  extending the harness — it is the single legitimate `manual` gate (C6).
- **F17 — Doc sites invalidated by this plan (swept in C8):** `AGENTS.md:33` (kernel module list contains `keyboard/`), `AGENTS.md:42` (app member list),
  `AGENTS.md:150` (`notification-driven` futures inventory "(time, task-completion, keyboard)"), `AGENTS.md:154` (ROM sentence "its roster holds only
  underscore-free names because the kernel keyboard drops shift state (see `docs/todo.md`)" and the stale ELF count); `docs/todo.md:32`
  (`## Deferred keyboard: shifted characters are not typeable (decided 2026-10-20)`); `README.md:174-181` (apps tree, insert lines in the `dummy/` region
  per the waitset-plan precedent). `README.md:72` ("Full IDT and PIC management for hardware interrupts (keyboard, timer)") stays **true** after C7 —
  the arch ports still manage the keyboard IRQ — and is not edited.
- **F18 — Known gap, out of scope:** task exit does not GC IPC connections or parked futures; a dead daemon stalls its consumers. This is the same
  pre-existing gap as `vfsd`; this plan neither fixes nor worsens it (a dead `inputd`/`ps2d` is behaviorally equivalent to today's wedged keyboard).

---

## 2. Treaty contracts (frozen; `input-protocol` crate + one `system` constant)

### 2.1 Crate `input-protocol` (workspace member, `no_std`, deps: `system` only; `pub mod wire;`)

`src/wire.rs` constants (all `pub`):

- `pub const SERVICE_NAME: &str = "INPUTD";`
- `pub const SOURCE_NAME_PS2KEYBOARD: &str = "PS2KEYBOARD";`
- Frame tags (`u8`): `TAG_READ = 1`, `TAG_READ_NOWAIT = 2`, `TAG_PUSH = 3`, `TAG_KEY = 4`, `TAG_NO_KEY = 5`.

Frames are plain `[u8; system::ipc::MESSAGE_PAYLOAD_BYTES]` arrays with `data[0]` = tag. Char frames (`TAG_PUSH`, `TAG_KEY`) carry the char as
`u32` little-endian in `data[1..5]`; all other bytes zero. Scancode frames (kernel → `PS2KEYBOARD`) carry the raw scancode in `data[0]` and zeros
elsewhere — **scancode frames carry no tag**; provenance is the conn word (§2.3), and `ps2d` is the only consumer of that mailbox.

Codec contract (pure fns, no allocation, prose over snippets):

- `encode_read() -> [u8; 60]`, `encode_read_nowait() -> [u8; 60]`, `encode_no_key() -> [u8; 60]` — zero arrays with the tag at index 0.
- `encode_push(c: char) -> [u8; 60]`, `encode_key(c: char) -> [u8; 60]` — identical bodies (tag differs); char via `c as u32` LE into `data[1..5]`.
- `frame_tag(data: &[u8; 60]) -> u8` — returns `data[0]`.
- `frame_char(data: &[u8; 60]) -> Option<char>` — `char::from_u32` over the LE word; `None` for a non-char word.
- `encode_scancode(scancode: u8) -> [u8; 60]`, `scancode_of(data: &[u8; 60]) -> u8` — byte at index 0.

Unit tests (host): tag positions, char LE round-trip, `frame_char` rejects a non-char word (e.g. `0xD800` surrogate encoded raw), every encoder leaves
exactly one tag plus zero padding (byte-multiset assertion), scancode round-trip.

### 2.2 Wire flows (binding, byte-for-byte)

- App → INPUTD: `TAG_READ` or `TAG_READ_NOWAIT` via `Syscall::ipc_send` (client role). INPUTD replies on the request's connection: `TAG_KEY` + char, or
  `TAG_NO_KEY`. The reply lands in the client mailbox by caller-identity routing (F8) and the app parks `ipc_receive_message` + `wait_for_message`.
- ps2d → INPUTD: `TAG_PUSH` + char via `Syscall::ipc_send` (also client role; INPUTD does not reply to pushes). INPUTD distinguishes request vs push
  solely by `frame_tag` — no device knowledge, no per-connection typing.
- Kernel → PS2KEYBOARD: scancode frame, conn = `KERNEL_EVENT_CONN`. Not reachable from any user `send_message` (§2.3).

### 2.3 `KERNEL_EVENT_CONN` (added to `system/src/ipc.rs`)

`pub const KERNEL_EVENT_CONN: u32 = 0x8000_0000;` — token-space analysis (F9): every real connection word is `< 2^31`, so the sentinel can never alias a
live connection, and it is distinct from `Message::EMPTY`'s conn `0`. It is **not** a token (`is_token` is `raw < ERROR_BIT`), therefore
`from_packed`/`from_token` reject it and `send_message` refuses to route it (the existing `send_on_error_bit_conn_word_rejected` test proves the refusal
path). Consequence, binding for C5/C6: `deliver_to_service` stamps it (kernel constructs the `Message` directly, bypassing `send_message`), and the
receiver discriminates with `ReceivedMessage::connection().is_none()` — user-forgery through the public send path is impossible. Treaty test in
`system/src/ipc.rs`: assert `!system::error::is_token(KERNEL_EVENT_CONN as usize)` and `KERNEL_EVENT_CONN != 0`.

---

## 3. Kernel design contracts

### 3.1 `deliver_to_service` (C5; additive, inert)

- `kernel/src/ipc/errors.rs`: add `pub(crate) enum DeliverError { ServiceNotFound, Congested }` with the same derives as the neighboring enums.
- `kernel/src/ipc/ipc_manager.rs`: `pub(crate) fn deliver_to_service(&mut self, service: &str, message: Message) -> Result<(), DeliverError>`.
  Behavior, in prose: look the name up in `registry`; absent → `ServiceNotFound`. Borrow the binding for that handle; borrow failure (stale) →
  `ServiceNotFound`. Push the message into the binding's **server mailbox** through `mailbox_manager.push_back`; map its `Ok(())` → `Ok(())` and
  `Err(ConnectionCongested)` → `Congested`. Pushing into a server mailbox is exactly what wakes a parked `accept_message_async` future (F8 chain);
  no new wait/wake mechanism is introduced.
- `kernel/src/ipc/mod.rs` (the `ipc` module is already `pub` in `lib.rs`): add
  `pub fn deliver_to_service(service: &str, message: system::ipc::Message) -> bool` — one-line wrapper over
  `services().ipc_manager.borrow_mut().deliver_to_service(...)` mapping `Ok` → `true`. `bool` keeps `DeliverError` `pub(crate)`; callers are arch ports
  in IRQ context and must simply drop `false` (bounded-drop policy, §6-4). This wrapper is the **only** new `pub` kernel surface in the plan.

### 3.2 Unit tests (in `ipc_manager.rs` test mod, mirroring existing `manager_with_notifier()` style)

1. `deliver_to_service_wakes_parked_accept`: bind + `accept_message_async` + `register_waiter` on a blocked task → deliver → task `Ready`, future holds
   the message with `conn() == KERNEL_EVENT_CONN`.
2. `deliver_to_unbound_service_is_service_not_found`.
3. `deliver_before_park_queues_message`: deliver first, then `accept_message_async` → future already completed with the message.
4. `deliver_congested_at_32`: 32 delivers Ok, the 33rd → `Congested`; after an accept, next deliver Ok (mirrors `test_congested_recovers_after_pop`).
5. `send_message_never_carries_the_kernel_event_conn`: a user-path `send_message` whose envelope conn is `KERNEL_EVENT_CONN` →
   `Err(IpcSendError::ConnectionNotFound)` (sentinel unforgeable; complements `send_on_error_bit_conn_word_rejected`).

### 3.3 Untouched (hard boundary, C5)

`future.rs`, `mailbox_manager.rs`, `mailbox.rs`, `keyboard.rs`, `messages.rs`, scheduler, `Kernel::enqueue` — all behave unchanged until C7 deletes the
dead pipeline. No `ErrorCode` variants (LD10). No `SchedulerFactory` signature change anywhere (§1 F4).

### 3.4 IRQ-context contract (binding for C6 call sites)

`deliver_to_service` may be called from interrupt context (both arches). The transitive work is `push_back` → `notify_waiters` →
`complete_ipc_message` + `notify` → `wake_tasks` (F8) — the same operation class the timer handler already performs inside `dispatch_irq` (F10).
The scancode frame must be constructed **before** the call (stack `Message::new`), and the return `bool` dropped — no logging, no blocking, no
`unwrap` on the IRQ path. A dedicated interrupt-guarded-sections pass (`Cpu` HAL guarding IPC/scheduler structures) is an explicit follow-up,
out of this plan (it equally affects the timer).

---

## 4. Daemon and usrlib contracts

### 4.1 `apps/inputd` (C2 adds crate + USER_APPS entry; boots at C4)

- `Cargo.toml` mirrors `apps/vfsd/Cargo.toml` verbatim in shape (`[lib] name = "inputd"`, `[[bin]] name = "inputd"` with `bench = false`,
  `test = false`; deps `system`, `usrlib`, `input-protocol`; the `cfg(target_os = "none")` runtime-feature block).
- `src/lib.rs` exposes `pub mod mux;` — pure host-testable state machine (imports only `core`/`alloc` + `input-protocol` + `usrlib` handle types):
  - `pub enum MuxAction { Reply(UserConnectionHandle, [u8; 60]) }`.
  - `pub struct InputMux` with two deques: `pending_reads` (FIFO of connection handles) and `buffered_chars` (FIFO `char`, cap 32, LD2).
  - `pub fn handle_read(&mut self, conn) -> Option<MuxAction>` — buffer non-empty → pop oldest and reply `TAG_KEY`; else enqueue `conn`, return `None`.
  - `pub fn handle_read_nowait(&mut self, conn) -> Option<MuxAction>` — buffer non-empty → pop oldest, reply `TAG_KEY`; else reply `TAG_NO_KEY`.
  - `pub fn handle_push(&mut self, c: char) -> Option<MuxAction>` — pending reader → pop oldest, reply `TAG_KEY`; else buffer, dropping the **oldest**
    char when at cap 32.
  - `pub fn forget(&mut self, conn)` — evict a dead reader from `pending_reads` (called when reply `ipc_send` failed).
- `src/main.rs` (boot body): `Syscall::ipc_bind(wire::SERVICE_NAME)` — `AlreadyBound` → sleep-and-retry (single-instance daemon; a second boot copy must
  idle, not panic), otherwise proceed; then loop: `ipc_accept_message` → `wait_for_message` → on `Ok(received)`: `received.connection()` required
  (a `None` conn word on the INPUTD binding is foreign — discard); decode `frame_tag`: `TAG_READ` → `handle_read`, `TAG_READ_NOWAIT` →
  `handle_read_nowait`, `TAG_PUSH` → `handle_push(frame_char(...))` (malformed → discard), unknown tag → discard; deliver each returned `MuxAction` with
  `Syscall::ipc_send` — on error, `forget` the connection. Follows `apps/vfsd/src/main.rs:86-95` reply-loop shape.
- Mux unit tests (≥ 9): read-then-key; key-then-read (buffered); nowait-hit; nowait-miss; two readers FIFO receive one char each in arrival order across
  two pushes; overflow drops oldest at 32; `forget` evicts and the next push skips the dead reader; malformed `frame_char` push leaves state unchanged;
  push never enqueues state for the pushing connection.

### 4.2 `apps/ps2d` phase A (C3; boots at C4) — bootstrap relay

- `Cargo.toml` mirrors `apps/ping/Cargo.toml` (binary-only, `test = false`).
- `src/main.rs`: connect-retry to `wire::SERVICE_NAME` (same shape and constants as `vfs-protocol/src/client.rs` F15, own copies per LD11), then
  forever: `let word = usrlib::arch::raw_syscall(SyscallNum::ReadChar as usize, 0, 0, 0);` → `char::from_u32(word as u32)` — `Some(c)` →
  `Syscall::ipc_send(connection, &wire::encode_push(c))` (on send failure: reconnect to INPUTD and continue) — `None` → continue. The sole legacy
  `ReadChar` consumer; safe against the single-waiter-slot limitation by being the only parker (C4 grep gate §5).

### 4.3 `apps/ps2d` phase B (C6) — real driver

- Reshape to `[lib]` + `[[bin]]` (vfsd shape F13): add `pub mod decoder;` to the lib — pure host-testable state machine owning the PS/2 set-1 tables
  ported out of `kernel/src/keyboard.rs` (`Key` enum `:61-149`, `from_scancode_set1` `:151-241`, char mapping `:249-337`; release-bit filter
  `scancode & 0x80 == 0` becomes part of the state machine, not a pre-filter).
- Decoder contract: `pub struct Ps2Decoder::new()`, `pub fn feed(&mut self, scancode: u8) -> Option<char>`. Behavior: left/right shift press
  (`0x2A`/`0x36`) and release (`0xAA`/`0xB6`) maintain shift state and produce no char; any scancode with bit 7 set produces no char (including shift
  releases); base table = today's `KeyboardEvent::from_key` chars; **LD4 shifted overlay** on press: letters uppercase; `1..0` → `!)@#$%^&*(`;
  `Minus`→`_`, `Equals`→`+`, `LeftBracket`→`{`, `RightBracket`→`}`, `Backslash`→`|`, `Semicolon`→`:`, `Apostrophe`→`"`, `Comma`→`<`, `Period`→`>`,
  `Slash`→`?`, `Grave`→`~`; non-char keys (`Escape`, modifiers, function keys, keypad) produce `None`. Extended-set (`0xE0`) sequences are out of scope
  (parity with today); the pending-byte state is not required because the current table ignores `0xE0` scancodes identically today.
- Decoder unit tests: unshifted letters/digits unchanged from the legacy table; the `docs/todo.md:32` sequence `0x2A 0x0C 0xAA` → exactly `'_'`
  (and nothing for the two shift frames); uppercase via shift + letter; release frames silent; unknown scancodes silent; shift release then plain press
  returns to base char.
- `src/main.rs` rewrite: bind `wire::SOURCE_NAME_PS2KEYBOARD` (AlreadyBound → sleep-and-retry) → loop: `ipc_accept_message` → `wait_for_message` →
  provenance check `received.connection().is_none()` (Some → foreign forged frame, discard) → `scancode_of` → `decoder.feed` → on `Some(c)` push
  `encode_push(c)` to INPUTD (reconnect-on-failure as phase A). INPUTD must be reachable first: boot order in both rosters (C4) schedules INPUTD before
  PS2D; the retry loop covers scheduling lag.

### 4.4 usrlib re-skin (C4; shared by both arch ports — reason for LD5)

- New private module `usrlib/src/input.rs` (not re-exported): holds a per-process cached `UserConnectionHandle` for INPUTD (module-scoped cell;
  usrlib is already the per-process glue layer — follow its existing statics discipline), a lazy connect with the F15 retry shape (50 attempts ×
  100 ms), and an invalidation path: if `ipc_send`/`wait_for_message` on the cached connection fails, drop the cache and re-resolve on the next call.
- `Syscall::read_char()` (signature unchanged, `usrlib/src/syscall.rs:82`): ensure connection → `ipc_send(conn, &wire::encode_read())` →
  `ipc_receive_message(conn)` → `wait_for_message` → `frame_char` on success; on any unrecoverable failure (INPUTD never connected / died) return
  `'\0'` — the same degraded value today's wrapper produces via `unwrap_or('\0')`.
- `Syscall::try_read_char()` (signature unchanged, `:87`): `encode_read_nowait` round-trip; `TAG_KEY` → `Some(char)`, `TAG_NO_KEY`/failure → `None`.
- The `SyscallNum::ReadChar` / `SyscallNum::TryReadChar` references in usrlib disappear entirely (grep gate at C4).

---

## 5. Steps (execution order; one commit per C-group, each green at its own boundary)

Branch shape: **C1–C4** on `inputd-ipc-bootstrap`; **C5–C8** on `ps2d-scancode-direct` (requires C4 merged). PR + squash merge per repo convention
(`(#NN)` suffix). Commit subjects (bare imperative): C1 "Add the INPUTD input treaty crate with the kernel event connection word";
C2 "Add the inputd mux daemon"; C3 "Add the ps2d keystroke relay"; C4 "Route application console input through INPUTD on both x86 ports";
C5 "Deliver kernel-originated events into bound IPC services"; C6 "Drive ps2d from raw PS/2 scancodes delivered at the keyboard IRQ";
C7 "Remove the legacy keyboard pipeline from the kernel"; C8 "Sweep input architecture facts into the documentation".

### Commit C1 — treaty only (additive, inert)

**Step 1.** `Cargo.toml`: add `"input-protocol",` to `members` after `    "vfs-protocol",` (`:6`) and to `default-members` after
`    "vfs-protocol",` (`:31`). Create `input-protocol/Cargo.toml` (mirror the `vfs-protocol` package block; dependencies: `system = { path = "../system" }`
only) and `input-protocol/src/lib.rs` with `pub mod wire;`.

**Step 2.** `input-protocol/src/wire.rs` per §2.1 + §2.2: constants, codecs, and the unit-test list (tag positions, LE round-trip, surrogate rejection,
zero-padding byte-multiset, scancode round-trip).

**Step 3.** `system/src/ipc.rs`: add `pub const KERNEL_EVENT_CONN: u32 = 0x8000_0000;` near `MESSAGE_PAYLOAD_BYTES` and the §2.3 treaty test
(`is_token` false; `!= 0`). No `ErrorCode` change (LD10) — the four inventory sites stay untouched.

**Gate C1:** `cargo xtask test --skip-integration`.

### Commit C2 — inputd daemon (inert: not on any roster)

**Step 4.** New `apps/inputd/Cargo.toml` (vfsd shape F13, deps `system`, `usrlib`, `input-protocol`), `src/lib.rs` (`pub mod mux;`),
`src/mux.rs` per §4.1 with the 9+ unit tests, `src/main.rs` accept-loop per §4.1.

**Step 5.** `xtask/src/main.rs:5`: append exactly `"inputd"` to the F11 verbatim list, yielding
`const USER_APPS: [&str; 13] = ["pi", "randomd", "echo", "bulk", "pong", "ping", "vfsd", "vfs", "snake", "tetris", "conway", "threads", "inputd"];`
(this adds the compile+ROM proof per F13; booting still excludes it — no roster change here).

**Gate C2:** `cargo xtask test --skip-integration` (runs the mux host tests via the workspace) + `cargo xtask build` (compiles the bin for the target).

### Commit C3 — ps2d bootstrap relay (inert)

**Step 6.** New `apps/ps2d/Cargo.toml` (ping shape F13, deps `system`, `usrlib`, `input-protocol`), `src/main.rs` per §4.2.

**Step 7.** `xtask/src/main.rs:5`: append `"ps2d"` →
`const USER_APPS: [&str; 14] = ["pi", "randomd", "echo", "bulk", "pong", "ping", "vfsd", "vfs", "snake", "tetris", "conway", "threads", "inputd", "ps2d"];`

**Gate C3:** `cargo xtask test --skip-integration` + `cargo xtask build`.

### Commit C4 — atomic user-facing cutover (LD9): usrlib re-skin + both rosters

**Step 8.** usrlib re-skin per §4.4 (new private `usrlib/src/input.rs`; rewrite `read_char`/`try_read_char` bodies at `usrlib/src/syscall.rs:82-95`
over the INPUTD wire; drop the now-unused `SyscallNum::ReadChar`/`TryReadChar` references and any import that becomes dead).

**Step 9.** `arch/x86_64/src/main.rs`: insert between the F14 anchors (`VFSD` schedule line `:88`, `SHELL_ELF` static `:89`) four lines following the
verbatim neighboring pattern: statics `INPUTD_ELF` / `PS2D_ELF` with
`include_bytes!("../../../target/rosx-user/release/inputd")` and `.../ps2d`, each followed by `let _ = kernel.schedule(new_elf_task(<name>));`,
INPUTD first, then PS2D, both **before** `SHELL_ELF` (schedule order = startup order).

**Step 10.** `arch/x86_32/src/main.rs`: insert above the F14 anchor (`:56`) the same two static/schedule pairs with the
`../../../target/rosx-i686-user/release/` path prefix and `kernel::task::new_elf_task` call form (matching `:57`), INPUTD before PS2D before SHELL.
Build prerequisite (doc-only per F14): inputd and ps2d ELFs exist under `target/rosx-i686-user/release/` (i686-user build of both apps with the `-Z`
flags from `xtask/src/main.rs:7-11` UNSTABLE list, per the AGENTS.md x86_32 recipe).

**Gate C4:** `cargo xtask build`; `cargo test -p tests-integration --test boot_and_shell` (`unverified` — local QEMU; proves the whole app→INPUTD chain
since `shell_echoes_keystrokes` types real keys); `cargo test -p tests-integration --test shell_command_arguments` (`unverified`);
grep gates (POSIX `grep -E`):
`grep -E "SyscallNum::(ReadChar|TryReadChar)" usrlib/src/syscall.rs` — empty;
`grep -E "SyscallNum::ReadChar" apps/ps2d/src/main.rs` — non-empty (the relay owns the legacy path now);
`grep -rE "Syscall::(read_char|try_read_char)" apps/shell/src apps/snake/src apps/tetris/src apps/conway/src` — non-empty unchanged (F7: apps untouched);
x86_32: `cargo build -p rosx-x86` from `arch/x86_32/` (`unverified`, no CI).

### Commit C5 — kernel event delivery (additive, inert)

**Step 11.** `kernel/src/ipc/errors.rs`: add `DeliverError` per §3.1.

**Step 12.** `kernel/src/ipc/ipc_manager.rs`: add `deliver_to_service` per §3.1 and the five unit tests per §3.2.

**Step 13.** `kernel/src/ipc/mod.rs`: add the `pub fn deliver_to_service(service: &str, message: system::ipc::Message) -> bool` wrapper (§3.1).

**Gate C5:** `cargo xtask test --skip-integration`.

### Commit C6 — atomic driver cutover (LD9): raw scancodes at IRQ time, ps2d decodes

**Step 14.** `arch/x86_64/Cargo.toml` + `arch/x86_32/Cargo.toml`: add `input-protocol = { path = "../../input-protocol" }` to `[dependencies]`
(both already depend on `system` — F baseline; `input-protocol` is `no_std` with a `system`-only dep, safe on bare metal).

**Step 15.** `arch/x86_64/src/interrupts.rs`: replace the F1 handler body: keep the `KEYBOARD_PORT` read into `scancode`; build
`Message::new(system::ipc::KERNEL_EVENT_CONN, &input_protocol::wire::encode_scancode(scancode))`; call
`kernel::ipc::deliver_to_service(input_protocol::wire::SOURCE_NAME_PS2KEYBOARD, message)`; drop the `use kernel::messages::HardwareInterrupt;` import
(`:8`). The IDT stub (`:117-119`), `register_handler` line (`:68`), `enable_keyboard()` (`:71-77`) and the timer path are untouched (IRQ lifecycle stays
inside the #77 dispatch core: mask → EOI → handler → re-arm).

**Step 16.** `arch/x86_32/src/interrupts.rs`: replace the F2 body in place: `inb(0x60)` → `PIC_EOI` outb (preserve the read-then-EOI order) →
`kernel::ipc::deliver_to_service(input_protocol::wire::SOURCE_NAME_PS2KEYBOARD, message)` with the same frame construction as Step 15; drop the
`use kernel::messages::HardwareInterrupt;` import (`:4`).

**Step 17.** `apps/ps2d`: reshape to `[lib]`+`[[bin]]` (F13); add `src/decoder.rs` per §4.3 (set-1 tables ported verbatim from
`kernel/src/keyboard.rs:61-337` — copy first, the kernel copy is deleted in C7, keep both green until then) with the decoder unit-test list;
rewrite `src/main.rs` to the bind/accept/decode/push loop per §4.3.

**Gate C6:** `cargo xtask test --skip-integration` (runs decoder tests; kernel delivery tests already green from C5); `cargo xtask build`;
`cargo test -p tests-integration --test boot_and_shell` (`unverified`); `cargo test -p tests-integration --test shell_command_arguments`
(`unverified`); `cargo build -p rosx-x86` from `arch/x86_32/` (`unverified`); **manual gate (justified per F16 — harness has no `-cpu` knob):** boot the
xtask disk image under `qemu-system-x86_64 -cpu qemu64,-apic ...`, confirm the fallback banner `[KERNEL] Interrupt controller: 8259 PIC` and that
keystrokes still echo; record the observation in the PR. A future harness `-cpu` knob is an explicit follow-up (§8).

### Commit C7 — delete the legacy pipeline (Appendix A is the checklist; locate sites by content, line numbers are orientation on `2f03e2f`)

**Step 18.** Delete `kernel/src/keyboard.rs` and `kernel/src/messages.rs`; in `kernel/src/lib.rs` remove `mod keyboard;` (`:20`) and
`pub mod messages;` (`:22`).

**Step 19.** `kernel/src/kernel.rs`: remove `KernelInterruptHandler` + `KERNEL_INTERRUPT_HANDLER` (`:69-87`), `Kernel::enqueue` (`:203-208`), and the
now-dead imports `use crate::messages::HardwareInterrupt;` (`:8`) and `use crate::ports::driven::ForHandlingHardwareInterrupts;` (`:15`).

**Step 20.** `kernel/src/ports/driven.rs`: remove `ForHandlingHardwareInterrupts` (`:43-45`) and its `HardwareInterrupt` import (`:6`).

**Step 21.** `kernel/src/scheduler/scheduler.rs`: remove `NoopInterruptHandler` + `NOOP_INTERRUPT_HANDLER` (`:35-38`), the `interrupt_handler`
field/param/noop wiring (`:62`, `:72`, `:87` — and its entry in the `SchedulerPorts::new` constructor body), the struct fields (`:99`, `:102`) and
their initializers in `Scheduler::new`, the `step()` drain preamble (`:117-120`) and postamble (`:129-130`), `push_hardware_interrupt` (`:150-152`),
`drain_hardware_interrupts` (`:209-213`), the three tests (`:441`, `:866`, `:975`) and the two handler impls (`:797`, `:816`) that exercise them, plus
dead `HardwareInterrupt`/`ForHandlingHardwareInterrupts` imports.

**Step 22.** `kernel/src/scheduler/mod.rs`: in `kernel_ports` remove the
`        &crate::kernel::KERNEL_INTERRUPT_HANDLER,` line (`:28`) — and the matching parameter is **not** part of `kernel_ports`' own signature; only the
`SchedulerPorts::new` arity changes, which no port file calls (F4).

**Step 23.** `kernel/src/syscall.rs`: remove the `ReadChar` (`:38-50`) and `TryReadChar` (`:77-79`) arms; `system/src/syscall_numbers.rs`: remove
`    ReadChar = 4,` (`:8`), `    TryReadChar = 9,` (`:13`), `            4 => Ok(Self::ReadChar),` (`:36`), `            9 => Ok(Self::TryReadChar),`
(`:41`) and add the retirement test `read_char_and_try_read_char_numbers_stay_retired` asserting `SyscallNum::try_from(4usize).is_err()` and
`SyscallNum::try_from(9usize).is_err()` (LD7).

**Gate C7:** `cargo xtask test --skip-integration`; `cargo xtask build`; `cargo test -p tests-integration --test boot_and_shell` (`unverified`);
`cargo test -p tests-integration --test shell_command_arguments` (`unverified`); `cargo build -p rosx-x86` from `arch/x86_32/` (`unverified`);
grep gates (POSIX `grep -E`, all expected **empty**):
`grep -rEl "HardwareInterrupt|ForHandlingHardwareInterrupts|KernelInterruptHandler|KERNEL_INTERRUPT_HANDLER|push_hardware_interrupt" kernel/src arch usrlib apps`;
`grep -rEl "ReadChar|TryReadChar" kernel/src system/src usrlib/src apps`;
`grep -rEl "keyboard" kernel/src` (the kernel holds zero keyboard knowledge after this step);
`test -f kernel/src/keyboard.rs || test ! -e kernel/src/keyboard.rs` style absence is covered by the greps above plus a build failure if a module ref
survives. Acceptance is expressed as **zero new warnings** (Δ-new = 0 vs the C6 boundary build log; decreases at deleted sites are expected, attributed
by location) — never absolute zero over a substrate with a non-proven baseline.

### Commit C8 — documentation sweep (§1 F17, binding corrections)

**Step 24.** `AGENTS.md`: `:33` drop ` keyboard/` from the kernel module list; `:42` append `inputd, ps2d` to the app member list; `:150` drop
`, keyboard` from the notification-driven futures inventory and extend the `ipc/` line with "kernel-originated event delivery
(`deliver_to_service`) wakes accepts on bound services"; `:154` rewrite the ROM sentence to drop the stale ELF count (state the roster as "the app ELFs
(not the shell)") and drop the shift-state clause, keeping the underscore constraint noted as historical with the rename follow-up (§8). Insert one
architecture bullet under "Kernel Subsystems": input policy lives in the user-space `INPUTD` daemon, device semantics in driver daemons (`ps2d`), the
kernel delivers opaque event frames only.

**Step 25.** `docs/todo.md`: mark the `:32` deferred-keyboard section resolved (decided 2026-10-25) — note the shift state machine landed in
`apps/ps2d` (LD4) and that ROM app renames remain a separate future task.

**Step 26.** `README.md`: insert `inputd/` and `ps2d/` tree lines in the apps region (after `│   ├── dummy/              # Dummy app for testing`,
`:181`, matching the surrounding column formatting); leave `:72` untouched (F17).

**Gate C8:** presence/absence `grep -E` over the edited files: `keyboard` absent from the AGENTS kernel-module line and the todo heading section
reports resolved; `inputd`/`ps2d` present in the AGENTS app list and README tree; `cargo xtask test --skip-integration` (docs-only; keeps the series
trivially green).

---

## 6. Risks and edge cases (Coder must handle)

1. **Boundary stranding (LD9).** usrlib is shared: re-skinning it without both rosters hangs every app's input; booting daemons without the re-skin
   leaves two `ReadChar` parkers racing on the single waiter slot (`kernel/src/keyboard.rs:15-19` — `replace`, so one parker starves). C4 is the single
   commit that moves both sides; likewise C6 moves arch + ps2d together. No intermediate boot may exist with a half-moved chain.
2. **Single-waiter legacy path is owned by ps2d by construction after C4.** The pre-existing latent starvation (shell, snake, tetris, conway all
   parking `read_char`, F7) is structurally eliminated by the re-skin; C4's grep gate pins sole ownership.
3. **Bounded queues and drop policy.** INPUTD's char queue: cap 32, drop-oldest (LD2). `PS2KEYBOARD` mailbox: cap 32 (`mailbox.rs:4`); on `Congested`
   the arch handler drops the keystroke (`deliver_to_service` returns `false`; an IRQ cannot block). Document both in the wire docstrings-as-test-names
   house style; no silent unbounded buffering anywhere on the new path (this also retires today's unbounded `VecDeque` keyboard ring).
4. **IRQ-context discipline (§3.4).** Same wake class as the timer handler (F10) — acceptable under current kernel conventions; interrupt-guarded
   sections remain a separate hardening follow-up (§8), affecting timer and IPC alike.
5. **Daemon death = input stall (F18).** Connections/futures are not GC'd on task exit (pre-existing, same as `vfsd`); a dead INPUTD leaves apps blocked
   (usrlib returns `'\0'`-degraded only for the never-connected case; a mid-life death blocks — parity with today's wedged keyboard). Respawn/watchpat is
   explicitly out of scope; do not add reconnect-on-death kernel machinery.
6. **`try_read_char` is best-effort.** A char in flight (already past INPUTD's queue into a kernel mailbox hop) may report none; no app on F7 depends
   on try semantics tighter than drain-until-empty loops.
7. **Sentinel aliasing.** Pinned by F9 facts + §2.3 tests: `0x8000_0000` is neither a token (`is_token`) nor `Message::EMPTY.conn` (0), and the public
   send path provably refuses it. Do not "reuse" `0` as the sentinel — arena slot 0 / generation 0 **packs to 0** and would alias the first live
   connection (`kernel/src/collections/arena.rs:26,32-37`).
8. **Boot-order and connect races.** INPUTD scheduled before PS2D before SHELL in both rosters (Steps 9-10); ps2d and usrlib retry-connect loops
   (50 × 100 ms, F15) cover scheduling lag. A `cargo run` boot with the daemons absent would spin retries, not panic.
9. **New app crates are host-compiled only as libs** (`test = false` bins, F13) — the lib/bin split is mandatory for the unit-test mandate to actually
   run in `cargo xtask test`; keep the mux/decoder logic out of `main.rs`.
10. **ROM roster grows by two** (`inputd`, `ps2d` — underscore-free, compliant with the pre-LD4 constraint); C8's AGENTS rewrite must not restate an
    ELF count (F17).

---

## 7. Acceptance criteria (whole plan)

| Command / gate | Status | Notes |
| --- | --- | --- |
| `cargo xtask test --skip-integration` | CI-proven form (`.github/workflows/ci.yml:31`) | every C-gate; runs new crate tests, mux tests, decoder tests, kernel delivery tests |
| `cargo xtask build` | CI-proven form (`ci.yml:51`) | compiles USER_APPS ELFs, ROM, kernel, disk image |
| `cargo test -p tests-integration --test boot_and_shell` | `unverified` — local QEMU (harness F16); routes to orchestrator baseline | end-to-end INPUTD proof via `sendkey` at C4/C6/C7 (LD12) |
| `cargo test -p tests-integration --test shell_command_arguments` | `unverified` — local QEMU | keyboard-driven; input-chain witness |
| `cargo build -p rosx-x86` from `arch/x86_32/` | `unverified` — doc-only recipe, no CI job (F14) | both arch ports green at C4/C6/C7 |
| POSIX `grep -E` gates (§5 C4/C7) | provable from file text | binding as written |
| PIC-fallback boot (`-cpu qemu64,-apic`) | `manual` — justified: harness exposes no `-cpu` knob (F16); knob-adding is a follow-up | C6 only; observation recorded in the PR |
| Warning gate | Δ-new = 0 vs the previous C-boundary build log, arbitrated by location first; decreases at deleted sites expected (C7) | never absolute-zero counts |

End-state proof: after C7, `grep -rEl "keyboard" kernel/src` is empty while `boot_and_shell` still types `"hello"` and sees it echoed —
IRQ → `dispatch_irq(1)` → arch frame → `deliver_to_service("PS2KEYBOARD")` → ps2d decode → `INPUTD` → shell — with zero keyboard semantics in `kernel/`.

---

## Appendix A — Legacy pipeline deletion inventory (C7; content-located, line refs at `2f03e2f`)

`kernel/src/keyboard.rs` (whole) · `kernel/src/messages.rs` (whole) · `kernel/src/lib.rs:20,22` · `kernel/src/kernel.rs:8,15,69-87,203-208` ·
`kernel/src/ports/driven.rs:6,43-45` · `kernel/src/scheduler/scheduler.rs:5,10,35-38,62,72,87,99,102,117-120,129-130,150-152,209-213` + test-mod sites
`:438-448,797-830,866,896,975,999-1000` · `kernel/src/scheduler/mod.rs:28` · `kernel/src/syscall.rs:38-50,77-79` ·
`system/src/syscall_numbers.rs:8,13,36,41` · `usrlib` legacy references (zero after C4 — verified by grep, not deleted here).

## Appendix B — New files manifest

`input-protocol/{Cargo.toml, src/lib.rs, src/wire.rs}` · `apps/inputd/{Cargo.toml, src/lib.rs, src/mux.rs, src/main.rs}` ·
`apps/ps2d/{Cargo.toml, src/lib.rs (C6), src/decoder.rs (C6), src/main.rs}` · `usrlib/src/input.rs` (C4).
Modified non-new files: `Cargo.toml` (members), `system/src/ipc.rs`, `system/src/syscall_numbers.rs`, `xtask/src/main.rs`,
`usrlib/src/syscall.rs`, `arch/x86_64/{Cargo.toml, src/main.rs, src/interrupts.rs}`, `arch/x86_32/{Cargo.toml, src/main.rs, src/interrupts.rs}`,
`kernel/src/ipc/{errors.rs, ipc_manager.rs, mod.rs}`, and the C7 deletion set; docs: `AGENTS.md`, `docs/todo.md`, `README.md`.

## Appendix C — Explicit follow-ups (recorded, not silently dropped)

Interrupt-guarded critical sections for IPC/scheduler structures via the `Cpu` HAL (timer + delivery alike); QEMU harness `-cpu` knob to script the
PIC-fallback gate; daemon respawn/watchpat (shared with vfsd); richer key-event wire shape when apps need arrows/release events (LD3 revisit);
ROM app renames enabled by the shift fix (LD4); `TryReadChar`-style zero-hop fast path only if profiling ever says typing matters (it will not).
