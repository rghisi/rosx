# tests-integration

Integration tests that build the RosX x86_64 kernel, create a BIOS disk image,
boot it in a QEMU instance spawned by the test, inject keystrokes, and assert
on guest output.

## Running

```
cargo test -p tests-integration
```

Requirements:

- `qemu-system-x86_64` on the `PATH`
- nightly toolchain with the `rust-src` component (pinned by `rust-toolchain.toml`;
  the kernel is built with `-Z build-std` against `arch/x86_64/rosx.json`)
- No KVM flag is passed, so tests run under pure TCG and are slow; boot
  assertions use generous 60 s timeouts
- Recommended: `cargo test -p tests-integration -- --test-threads=1` to avoid
  TCG CPU contention

The kernel is built once per test run (guarded by a `OnceLock`) by shelling out
to cargo against `arch/x86_64/Cargo.toml`.

## Environment variables

| Variable | Meaning |
|---|---|
| `ROSX_QEMU_PROFILE` | `dev` or `release`; opt-in kernel build when `ROSX_KERNEL_ELF` is unset (unset both and the harness fails with a message); any other value panics |
| `ROSX_KERNEL_ELF` | absolute path to a prebuilt kernel ELF; skips the kernel build (`cargo xtask test` builds the release kernel and sets it) |
| `ROSX_QEMU_KEEP_TMP` | if set to a non-empty value, the session temp dir (disk image, monitor socket) is kept on drop and its path is printed to stderr |
| `ROSX_BOOT_SAMPLES` | sample count for the boot-to-prompt measurement test (default 5) |

## Measuring boot time

`tests/measure_boot.rs` is `#[ignore]`d so it never joins a normal suite run; it samples
the boot-to-prompt distribution — session spawn (image creation + exec), `[KERNEL]
Initializing`, `[KERNEL] Starting`, and the `rose>` prompt, all from one start instant:

```
cargo test -p tests-integration --test measure_boot -- --ignored --nocapture
```

The measured baseline and the speed-up plan built on it live in
`plans/integration-baseline-2026-11-07.md`.

## Shared sessions

A plain `QemuSession` is one boot: its whole value is the running shell, so a second test in the same
binary pays for another boot. `tests_integration::shared::SharedSession` fixes that: one binary owns one
VM and runs a fixed list of named cases on it, paying for a single boot.

```rust
let mut group = SharedSession::spawn("my_group");
group.case("name_a", case_a);
group.case("name_b", case_b);
```

- Cases run in the listed order on one session, so a case sees what the earlier ones did: order by the
  boot sequence, prompt state and `ps` set, not by convenience.
- After every case except the last, `SharedSession` resets the machine: Ctrl-C, a fresh prompt, and a
  process-set compare against the boot baseline. A failed reset is a hard failure naming the group and
  the last case, because the next case's failure would otherwise be a mystery.
- A case that never returns control (it loops, or waits on something that never happens) must be ordered
  last, where no reset follows it.
- The last `case` call returns the group by value; dropping it quits and reaps QEMU, so a panicking case
  cannot leak a VM.

Grouped cases keep their old test names inside `case("...")`, so coverage stays readable in the file.
Assertions that measure absolute bytes across many cycles (the `kstats_mem` erosion tests) stay on their
own VMs: they are the measurements a reset would contaminate.

## Tests

| File | Shape |
|---|---|
| `boot_and_exit_status.rs` | one VM: boot and disk-mount proof, the memory snapshot table, NIC-less init diagnostics, shell echo/`ls`/unknown command, and the exit-status family (11 cases) |
| `ctrl_c.rs` | one VM: Ctrl-C kills the foreground app, redraws at a bare prompt, discards a partial line, spares background jobs (4 cases; the spinner loop is last) |
| `ps_listing.rs` | one VM: the four `ps` table cases (4 cases) |
| `shell_background.rs` | one VM: background launch, foreground-waits, panic report, built-in ignored, reap-exactly-once (5 cases) |
| `shell_single_shots.rs` | one VM: cases that need a pristine prompt and nothing else — the `mem` snapshot, `pi` argv paths, `leaky` future forfeiture, `waitset` composition (5 cases) |
| `kstats_mem.rs` | two VMs: used-bytes erosion across repeated `mem` runs and load-run-kill cycles |
| the remaining files | one VM per `#[test]`, as before |

## How it works

- Guest output: QEMU runs with `-debugcon stdio`, so everything the kernel
  writes to the debug console arrives on the QEMU child's stdout pipe and is
  collected into a shared, ANSI-stripped buffer.
- Keystroke injection: QEMU is started with `-monitor
  unix:<tempdir>/monitor.sock,server=on,wait=off`; characters are mapped to
  `sendkey` tokens and sent one per command. Each keystroke is acknowledged by
  its echo on the debug console before the next one is sent, and resent up to
  three times before the test panics; `send_line` finishes with a plain
  (un-echoed) Enter.
- Each `#[test]` owns its `QemuSession` (a `SharedSession` owns exactly one): private temp dir (disk image
  + monitor socket) and private QEMU child; on drop the session sends monitor `quit`, reaps the child, and
  deletes the temp dir unless `ROSX_QEMU_KEEP_TMP` is set. Run the suite with `--no-fail-fast` to see every
  group's verdict in one pass.
