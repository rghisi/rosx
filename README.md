# RosX

RosX is a hobby operating system written in Rust, with minimal assembly. It started as a way to learn Rust and experiment with OS design. Since late 2026 it is also a test bed for **agentic AI coding** and for **running local AI models**: most changes since then are written by a local Qwen3.8-Flash-Next model working through the repository's gates and plans, with the maintainer reviewing.

## Goals

- **Run without an MMU.** The target is small, simple devices; the stretch goals are a Sega Mega Drive and a Sony PlayStation 1.
- **Portable, microkernel-inspired design.** A lean, platform-agnostic `no_std` kernel; drivers, filesystems and the network stack run as user-space daemons.
- **Network applications.** An Ethernet/ARP/IP stack exists; the aim is to grow it into something that can serve real network applications.
- **A good environment for coding agents.** Strict, mechanical gates and written invariants, so a small local model can make safe changes.

## Status

| Area | State |
| --- | --- |
| x86_64 | Complete; the bootstrap platform |
| x86_32 | Complete; the 486 is a maintained baseline |
| ARM, RISC-V, m68k, PlayStation 1, Mega Drive | Future; no `arch/` directories yet |
| Kernel | Preemptive MLFQ scheduling, IPC and futures, ELF user space |
| Storage | Block layer, VFS, FAT/RxFS/RAM filesystems, ROM initrd boot |
| Network | Ethernet (NE2000), ARP, IP |
| User space | Shell, text editor, input daemons, games and test apps |

## How this repo is developed

- [`AGENTS.md`](AGENTS.md) is the context document for coding agents: commands, invariants, working rules and autonomy limits. `CLAUDE.md` and `GEMINI.md` point to it.
- `cargo xtask check` is the gate: style rules, architecture rules, the 486 ISA audit, builds and unit tests. A change that fails it is not committed.
- `cargo xtask verify` is the merge evidence: the gate plus the full QEMU integration suite on both architectures.
- [`plans/`](plans/) holds the design notes and step-by-step plans agents work from.
- [`docs/decisions/`](docs/decisions/) records open decisions, [`docs/todo/`](docs/todo/) holds out-of-scope work, and [`docs/rationale/`](docs/rationale/) explains low-level `unsafe` code.

## Quick start

Requirements: Rust nightly with `rust-src` (the pinned toolchain installs itself), QEMU, and `mtools` for the FAT boot disk.

```bash
git clone https://github.com/rghisi/rosx.git
cd rosx
cargo xtask run             # build and boot x86_64 in QEMU to the shell prompt
cargo xtask run x86_32      # the same on the 32-bit port
cargo xtask check           # the gate
cargo xtask verify          # gate + full integration on both architectures
cargo xtask help            # every subcommand
```

`cargo build --workspace` is intentionally unsupported: the bare-metal binaries and PIE apps cannot compile for the host. Use `cargo xtask`.

## Architecture

The workspace is split by directory:

- `kernel/` is platform-agnostic and `no_std`. It depends only on `system`, and reaches hardware through HAL traits.
- `system/` is the ABI treaty: syscall numbers, wire layouts and error numbers.
- `arch/` holds the per-platform code, plus the bootloader and the image runner shared by the x86 ports.
- `usrlib/` is the user-space library; `apps/` holds the daemons and applications.
- `protocol/` holds the wire protocols between daemons, and `fs/` the filesystem engines.

Design documents are in [`docs/architecture/`](docs/architecture/): scheduling and interrupts, IPC and futures, memory, boot plan, disk and VFS, networking, input, the shell, and the 486 baseline.

## Releases

Boot images are published in [Releases](https://github.com/rghisi/rosx/releases) and as artifacts of the manual `Build Artifacts` workflow (kept for one day). Each is a unified boot disk: stage1 chain-loads stage2, which loads `KERNEL.ELF` from the FAT volume.

```bash
qemu-system-x86_64 -drive format=raw,file=rosx-boot-disk.img -debugcon stdio -no-reboot -no-shutdown
qemu-system-i386 -cpu 486 -drive format=raw,file=rosx-i486-boot-disk.img -debugcon stdio -no-reboot -no-shutdown
```

- `-debugcon stdio` carries console output over QEMU's debug port into your terminal; keyboard input goes to the VGA window.
- `-cpu 486` is the x86_32 baseline.
- `-no-reboot -no-shutdown` makes a guest panic halt visibly.

To try the block and filesystem stack, attach a second disk with `-drive if=ide,index=1,media=disk,format=raw,file=hdd.img` (create it with `truncate -s 50M hdd.img`), then in the shell run `ata list`, `attach <dev-id> ATA 1`, `format <dev-id>`, `mkdir <dir>`, `mount <dev-id> <dir>`, `ls <dir>`.

## License

MIT.
