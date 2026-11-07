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
| `ROSX_QEMU_PROFILE` | `dev` (default) or `release`; any other value panics |
| `ROSX_KERNEL_ELF` | absolute path to a prebuilt kernel ELF; skips the kernel build |
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

## Tests (`tests/boot_and_shell.rs`)

| Test | Purpose |
|---|---|
| `kernel_boots` | boot reaches `[KERNEL] Initializing` and `[KERNEL] Starting` |
| `shell_banner_and_prompt` | shell prints `ROSE Shell` and the `rose>` prompt |
| `shell_echoes_keystrokes` | typed text (`hello`) is echoed back by the shell |
| `shell_ls_and_unknown_command` | `ls` lists apps (`snake`, `tetris`) and `foo` yields `Unknown command: foo`; assertions scan only output appended after a captured mark |

## How it works

- Guest output: QEMU runs with `-debugcon stdio`, so everything the kernel
  writes to the debug console arrives on the QEMU child's stdout pipe and is
  collected into a shared, ANSI-stripped buffer.
- Keystroke injection: QEMU is started with `-monitor
  unix:<tempdir>/monitor.sock,server=on,wait=off`; characters are mapped to
  `sendkey` tokens and sent one per command with a 50 ms inter-key delay.
- Each test owns its `QemuSession`: private temp dir (disk image + monitor
  socket) and private QEMU child; on drop the session sends monitor `quit`,
  reaps the child, and deletes the temp dir unless `ROSX_QEMU_KEEP_TMP` is set.
