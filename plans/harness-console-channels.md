# Test Console Channels — Output Channel and Key Injector Seams

> **Status:** proposed, not started. Written 2026-11-21.
> **Branch base:** `main` at `54034e0` (#186). Every milestone applies to this base directly.
> **Feature:** the integration harness stops hard-wiring *how the guest's console reaches the test*
> and *how keystrokes reach the guest*. Two small seams inside `tests-integration` — an
> `OutputChannel` and a `KeyInjector` — carry today's x86 behaviour (debugcon output, PS/2
> keystrokes through the QEMU monitor) unchanged, and make a second implementation (a UART
> over QEMU's stdio) an additive change instead of a rewrite of `QemuSession`.
> **Independent of:** `plans/platform-runner-abstraction.md`. Neither plan needs the other;
> either may land first (§7 names the one shared file and how to resolve it).
> **Method:** every repo fact below was verified against this working tree (paths cited).
> Commands this plan cannot prove green are marked **`unverified`**.

---

## 1. Goal & Non-Goals

### Goal
- Separate the two halves of console I/O that `QemuSession` fuses today, so that:
  - the x86 pair (debugcon out, `sendkey` in) is one implementation among several;
  - a future arch whose only console is a UART (RISC-V `virt`, §3.5) adds an implementation,
    not a branch inside `spawn_capturing`;
  - the session logic that is pure — keystroke pacing, echo-ack retry, `run_command`
    repetition, mark/window arithmetic — becomes unit-testable against a fake console with
    no QEMU in the loop (today it has no unit tests at all, §2.7);
  - the integration crate's own unit tests run in the gate (they do not today, §2.6).

### Non-Goals
- **No behaviour change on x86.** Same QEMU flags, same monitor traffic, same retry constants.
  The 38 test files do not change; `QemuSession`'s public methods keep their signatures.
- **No UART implementation in this plan.** No kernel arch has a UART, so an implementation
  would have no production caller, and AGENTS.md forbids test-only production surface
  (§3.6). The UART design is specified (§3.5) and recorded as a todo with a trigger (M4).
- **No kernel, `system`, protocol, app or ABI change.**
- **No runner or `x86-runner` change.** Who adds `-debugcon stdio` to the base command stays
  with the runner (§2.4); the seam is written around that fact.
- **No migration of x86 to a UART.** Recorded as a decision with a revisit trigger (M4).

## 2. Verified baseline facts (2026-11-21, paths)

1. **Output is stdout-as-debugcon.** `x86_runner::qemu_command` adds `-debugcon stdio
   -no-reboot` (`arch/x86-runner/src/lib.rs:214`). `QemuSession::spawn_capturing`
   (`tests-integration/src/qemu.rs:172`) pipes stdout and feeds it to `SharedOutput` through
   `output::spawn_reader`; stdin is `Stdio::null()`; stderr is tailed separately for
   diagnostics.
2. **Other channels are switched off by the harness.** `spawn_capturing` appends
   `-monitor unix:<tmp>/monitor.sock,server=on,wait=off -serial none -parallel none`.
3. **Input is monitor `sendkey`.** `send_key` (`qemu.rs:310`) maps a `char` through
   `sendkey::token_for` (`tests-integration/src/sendkey.rs`, PS/2-oriented QEMU key names;
   arrows and Escape come from the `input_protocol::wire` sentinels) and sends
   `sendkey <token>` over the monitor; an unmappable character panics.
4. **Echo-ack retry.** `send_key_acked` (`qemu.rs:318`) re-sends up to `KEY_ATTEMPTS = 3`
   times, waiting `ECHO_ACK_CAP = 500 ms` for the character to appear in the output, then
   panics with the whole transcript. `send_text`, `send_line` and `run_command` build on it.
5. **One monitor connection.** `monitor_mut` (`qemu.rs:367`) connects lazily; `quit_qemu`
   (`qemu.rs:415`, called from `Drop` and `shutdown`) reuses it or connects with a 5 s cap.
   A QEMU unix chardev server serves one client at a time (QEMU behaviour, from memory:
   **`unverified`** here), so the injector and the shutdown path must share one connection.
6. **The gate skips this crate's unit tests.** `xtask/src/main.rs:249` runs
   `cargo test --workspace --exclude tests-integration` when `--skip-integration` is in
   force, which is exactly what `cargo xtask check` uses. The unit tests already in the crate
   (`arch.rs`, `sendkey.rs`, `output.rs`, `kernel_build.rs`, `bench_history.rs` and the three
   in `qemu.rs:466`) therefore run only inside the 15-40 minute integration passes. Baseline,
   run for this plan: `cargo test -p tests-integration --lib` builds without QEMU or
   `ROSX_KERNEL_ELF` and reports **42 passed, 0 failed**.
7. **The session logic has no fake-driven tests.** `qemu.rs`'s only unit tests cover
   `should_keep_temp_dir`; pacing, retry and `run_command` can only be exercised against QEMU.
8. **The test-facing surface is narrow** (call counts across `tests-integration/tests/*.rs`):
   `expect_output` 105, `expect_output_since` 94, `mark` 64, `send_line` 53, `content_since`
   31, `send_text` 11, `run_command` 9, `wait_until_since` 7, `temp_dir` 6, `shutdown` 6,
   `send_key` 6 (Ctrl+C ×4, Enter, and the ted keys). Only `ted_edit.rs` reaches for
   `temp_dir`.
9. **No kernel arch has a UART.** `KernelOutput` implementations are VGA text and the 0xE9
   debug console (`arch/x86-generic/src/{vga_buffer,debug_console}.rs`); the only COM1 write
   is in the x86 bootloader's stage 2 (`arch/x86-bootloader/stage2/src/entry.rs:120`).
10. **Repo rules this plan honours** (AGENTS.md): no comments; no test-only production
    surface (every new item has a non-test caller, §3.6); `#[cfg(test)]` scaffolding lives in
    the unit under test; commit messages carry no prefix; every step ends with the `retro`
    skill's step-end report; nothing is skipped or loosened to get a gate green.

## 3. Design decisions

### 3.0 Decisions taken (from review, 2026-11-21)

| # | Decision | Source |
|---|---|---|
| D1 | Two seams (output channel, key injector), not one console trait | maintainer |
| D2 | The UART implementation uses a stdio chardev, not a Unix socket | maintainer |
| D3 | No throughput spike before RISC-V work; the risk is accepted and recorded (§7) | maintainer |
| D4 | x86 keeps debugcon + `sendkey`; moving it to a UART is a later, recorded decision | default under the autonomy rules |
| D5 | The UART implementation is specified here but built with its first caller, not in this plan | default under the no-test-only-surface rule |

### 3.1 Why two seams
Today's x86 pair is (debugcon out, PS/2 in). RISC-V is (UART out, UART in). A third pairing —
(debugcon out, UART in) — is the natural first step if x86 ever adopts a serial console. The
halves vary independently, so they are separate traits composed per platform.

### 3.2 The seams (module `tests-integration/src/console/`)

```
trait OutputChannel          // how the guest's console text reaches SharedOutput
    qemu_args(&self) -> Vec<OsString>
    start(&self, child: &mut Child, sink: SharedOutput)

trait KeyInjector            // how a keystroke reaches the guest
    qemu_args(&self) -> Vec<OsString>
    inject(&mut self, control: &mut QemuControl, c: char) -> Result<(), InjectError>
    delivery(&self) -> Delivery          // Acked { attempts, cap } | Direct

struct Console { output: Box<dyn OutputChannel>, keys: Box<dyn KeyInjector> }
```

- **The key vocabulary stays `char`.** Arrow keys and Escape remain the
  `input_protocol::wire` sentinels, so no test call site changes. Each injector owns its own
  encoding: the monitor injector keeps `token_for`; a UART injector would emit the byte or an
  ANSI sequence (§3.5).
- **`qemu_args` returns data, not a mutated `Command`**, so unit tests can assert the exact
  arguments (the same habit as the arg-vector tests in `x86-runner/src/lib.rs:377-536`).
- **`Delivery`** carries the retry policy that is today hard-wired in `send_key_acked`: the
  monitor injector reports `Acked { attempts: 3, cap: 500 ms }` (the existing constants); a
  UART injector will report `Direct`, because QEMU flow-controls a serial chardev.
- **Trait objects, not generics**: the choice is made at runtime from the selected arch, and
  the call frequency (one call per keystroke) makes dispatch cost irrelevant.

### 3.3 `QemuControl` — the monitor, owned once
The lazily connected monitor and `quit` move out of `QemuSession` into `QemuControl`
(`tests-integration/src/control.rs`). `QemuSession` owns it; injectors borrow it per call.
This respects §2.5: exactly one connection, shared by keystrokes and shutdown. A UART injector
ignores it; `quit` stays the shutdown path on every arch.

### 3.4 Splitting the session
`QemuSession` currently mixes process lifecycle (child, stderr tail, temp dir, reaping) with
console interaction (marks, waits, keystrokes). The interaction half becomes `ConsoleIo`
(`console/io.rs`): it owns `SharedOutput`, the `Box<dyn KeyInjector>` and the pacing logic, and
exposes `mark`, `expect_output*`, `wait_until_since`, `content_since`, `send_key`,
`send_text`, `send_line`, `run_command`. `QemuSession` keeps every public method by delegating.
`ConsoleIo` can be built over a `SharedOutput` the test feeds by hand and a fake injector —
that is the unit-test seam.

### 3.5 The UART design (specified now, built with its first caller)
Recorded so the RISC-V port plan does not re-derive it:
- **Arguments:** `-display none -chardev stdio,id=rosxcon,signal=off -serial chardev:rosxcon`.
  Not `-nographic`: it multiplexes the monitor onto the same stream and QEMU then swallows
  Ctrl+A (`0x01`), a character the tests send. `signal=off` keeps Ctrl+C (`0x03`) a byte.
- **One stream, both directions.** The session pipes the child's stdin as well as stdout; the
  output channel reads the child's stdout, the injector writes the child's stdin. The
  control monitor stays a separate `-monitor unix:` socket.
- **Preconditions on the base command:** it must not also claim stdio (no `-debugcon stdio`,
  which is the runner's x86 contribution, §2.1) — QEMU refuses two stdio chardevs. The hybrid
  (debugcon out, UART in) therefore needs a Unix-socket chardev for the input leg; that is not
  built and not needed for the RISC-V pairing.
- **Encoding:** printable ASCII and control bytes pass through unchanged; Enter is `\n`
  (the guest's UART daemon maps CR/LF); backspace is `0x08`; Up/Down/Right/Left are
  `ESC [ A`, `ESC [ B`, `ESC [ C`, `ESC [ D`; a lone Escape is `0x1B`. Because a lone Escape
  and the start of a sequence are ambiguous to the guest decoder, the injector must follow a
  lone Escape with no further bytes until the test's next wait; the guest-side decoder's
  timeout is owned by the port plan.
- **Delivery:** `Direct`.
- **Wiring test:** a `cat`-backed unit test (a child that echoes stdin to stdout) proves the
  stdin write path, stdout read path, ANSI encoding and no-reordering with no QEMU. It lands
  with the implementation, because it is exercised by no production code until then (§3.6).

### 3.6 Why the UART implementation is not built here
AGENTS.md: *every production item has a non-test caller; production API with no caller is
deleted.* A `UartStdio` console selected by no arch would be exactly that. The seams built
here pass the rule: both have a production caller (`QemuSession`'s x86 console), and the fake
used by the new unit tests lives in the `#[cfg(test)]` module of the unit under test. When the
first riscv64 `QemuSession` exists, the UART implementation and its `cat` wiring test land in
that same change. Until then the design is a `docs/todo/` file with that trigger (M4).

A trait with one production implementation is a deliberate, bounded cost: it is justified by
the unit-test seam it buys today and the second implementation scheduled behind it.

## 4. What moves, what stays

| Piece | Verdict |
|---|---|
| `SharedOutput`, `strip_ansi`, `output::spawn_reader` | **stay**, untouched; implementations push bytes into them |
| `-serial none -parallel none`, stdout reader | **move** from `spawn_capturing` into `DebugconStdout` |
| `Monitor` (`monitor.rs`) | **stays**; `QemuControl` wraps it |
| `monitor_mut`, `quit_qemu`, `monitor_socket` | **move** into `QemuControl` |
| `send_key`, `send_key_acked`, `send_text`, `send_line`, `run_command`, `mark`, `expect_*`, `content_since`, `wait_until_since` | **move** into `ConsoleIo`; `QemuSession` delegates, signatures unchanged |
| `sendkey.rs` token table | **stays**, becomes the private encoder of `MonitorSendkey` |
| `-debugcon stdio`, `-no-reboot`, `-display none` in the base command | **stay** with `x86_runner::qemu_command` |
| `-monitor unix:…` | **stays** in `spawn_capturing` (it is session control, not console) |
| The 9 `spawn_*` constructors | **stay**; each picks `Console::for_arch(Arch::current())` |
| All `tests/*.rs` | **unchanged** |

## 5. Milestones (each one commit; approve-and-go per step)

Every milestone ends with the `retro` skill's step-end report in the commit report. The friction
log lives in `plans/harness-console-channels-impl.md`, created when M1 starts.

### M1 — Put the crate's own unit tests in the gate
- `xtask`: the check stage that runs unit tests also runs
  `cargo test -p tests-integration --lib -- --test-threads=1` (the lib target needs no QEMU and
  no `ROSX_KERNEL_ELF`). The baseline is 42 passing tests (§2.6); record the count after the
  stage lands in the impl log.
- **Gate:** `cargo xtask check` green with the new stage visible; **deliberate failure proof**:
  temporarily break one assertion in `sendkey.rs`, observe the gate fail naming the stage,
  revert.
- **Commit:** `Run the integration crate's own unit tests in the gate`

### M2 — `QemuControl` and the output channel
- New `control.rs` (`QemuControl`: lazy monitor connection, `send_command`, `quit`) and
  `console/mod.rs` with `OutputChannel` and `DebugconStdout`.
- `spawn_capturing` takes a `Console`; the stdout reader and `-serial none -parallel none`
  now come from `DebugconStdout`; `quit_qemu` goes through `QemuControl`.
- Unit tests: `DebugconStdout::qemu_args` equals exactly `["-serial", "none", "-parallel",
  "none"]`; `QemuControl` reports a clear error for a missing socket (existing
  `Monitor::connect_with_cap` is already testable with a temp path).
- **Gate:** `cargo xtask check`; `cargo xtask test` (x86_64) and
  `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1` (x86_32), both
  green. This is the oracle: any flag drift fails loudly.
- **Commit:** `Move console capture and monitor ownership out of QemuSession`

### M3 — The key injector, `ConsoleIo` and the fake
- `console/keys.rs`: `KeyInjector`, `Delivery`, `InjectError`, `MonitorSendkey` (wraps
  `sendkey::token_for` and `QemuControl`; reports `Acked { 3, 500 ms }`).
- `console/io.rs`: `ConsoleIo` with the moved pacing logic; `QemuSession` delegates.
- Unit tests on a fake injector and a hand-fed `SharedOutput` (in `console/io.rs`'s
  `#[cfg(test)]`): characters arrive in order; a dropped keystroke is retried and the retry
  succeeds; exhausting the attempts panics naming the character and the attempt count;
  `Delivery::Direct` never waits for an echo; `run_command` re-sends until `done` holds and
  stops at `Unknown command`; `mark`/`content_since` window correctly across pushes; the
  monitor injector turns an unmappable character into the existing panic message.
- **Gate:** as M2, plus the new tests visible in the M1 stage.
- **Commit:** `Split key injection and console pacing out of QemuSession`

### M4 — Selection, documentation and recorded deferrals
- `Console::for_arch(Arch)` returns `(DebugconStdout, MonitorSendkey)` for both x86 arches;
  the nine constructors use it.
- `docs/architecture/boot-media-and-harness.md`: a short "Test console channels" paragraph and
  one invariant line (console output and keystrokes reach the harness only through
  `OutputChannel` / `KeyInjector`; x86 uses debugcon and monitor `sendkey`).
- `docs/todo/harness-uart-console.md` (trigger: the first riscv64 `QemuSession`; content:
  §3.5 by reference to this plan) and `docs/decisions/harness-console-uart-for-x86.md`
  (`status: open`, `revisit-when:` an x86 serial console ships as a product feature, because
  the PS/2 path would then lose its only integration coverage unless a dedicated keyboard test
  set is kept).
- **Gate:** `cargo xtask check` (including `cargo xtask decisions` listing the new records),
  then `cargo xtask test` for both arches.
- **Commit:** `Select the test console per arch and record the UART follow-ups`

## 6. Deferred, recorded not dropped
- **`UartStdio` implementation and its `cat` wiring test** — `docs/todo/harness-uart-console.md`.
- **x86 on a UART** — `docs/decisions/harness-console-uart-for-x86.md`. Needs an x86 UART
  driver and a serial input daemon in the product; justified only as a real serial-console
  feature (no test-only surface).
- **Hybrid (debugcon out, UART in)** via a Unix-socket chardev — only if the decision above
  moves; not designed beyond §3.5's precondition note.
- **Keyboard-device injector (virtio-input + `sendkey`)** — rejected for the first RISC-V
  milestone (needs a driver and a decoder); the trait would take it unchanged.

## 7. Risks

| # | Risk | Mitigation |
|---|------|------------|
| 1 | x86 sessions drift (flag order, lost output, shutdown hang) | The seams change who owns the code, not what runs; M2 and M3 gate on the full integration suite for both arches; arg vectors asserted exactly |
| 2 | The shared monitor connection is dropped or used twice | One owner (`QemuControl`); the injector only borrows it; M3's tests include the shutdown-after-keystrokes path via the fake control seam |
| 3 | UART output throughput: QEMU's serial model may pace transmit at the programmed baud rate, which could slow kernel logging in heavy tests | **Accepted by the maintainer (D3).** Not measured. If it bites, the seam lets a platform keep a faster output channel and use the UART for input only; benchmarks stay on x86 |
| 4 | A one-implementation trait reads as speculative generality | Bounded by §3.6: a real caller exists, a unit-test seam is delivered now, and the second implementation is named and triggered |
| 5 | `qemu.rs` conflicts with the runner plan | The only shared file. The plans touch disjoint parts (this one: console and pacing; the other: command assembly and constructors). Whichever lands second rebases; neither plan's gates depend on the other |
| 6 | The new gate stage lengthens `cargo xtask check` | The lib unit tests need no QEMU; measured in M1 and recorded |

## 8. Verification matrix

| Command (cwd) | Proves |
|---|---|
| `cargo xtask check` (root) | the gate, now including `tests-integration --lib` (M1+) |
| `cargo test -p tests-integration --lib -- --test-threads=1` (root) | the unit tests, including the fake-console tests, in isolation |
| `cargo xtask test` (root) | x86_64 QEMU suite through the seams (M2-M4) |
| `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1` (root, `ROSX_KERNEL_ELF` set) | x86_32 QEMU suite through the seams |
| `cargo xtask verify` (root) | merge evidence for the PR series (~40 min) — **`unverified`** until run at the final head |

The authoring environment has no QEMU, so the QEMU rows are **`unverified`** until run by the
executor.

## 9. Interfaces summary
After M4 `tests-integration` gains `console::{OutputChannel, KeyInjector, Delivery,
InjectError, Console, ConsoleIo, DebugconStdout, MonitorSendkey}` and `control::QemuControl`.
`QemuSession`'s public methods, the `ROSX_*` environment variables, every test file, the
`x86-runner` API, xtask's user-facing commands, the kernel, `system`, the protocols and the
apps are unchanged. No ABI or wire change anywhere in this plan.
