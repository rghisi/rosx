# Operational gotchas

Every rule here was earned once in the exploratory session
(plans/x86_32-i486-impl.md, M5 supplement; blueprint
plans/x86-86box-integration-plan.md §2). Violating one costs a confused
hour, not a CI failure — CI cannot see any of this.

## Host & containers

- Wrap every `docker` invocation that runs in an agent shell in
  `sg docker -c '...'`. A running agent session's process groups do not
  pick up a `usermod -aG docker` grant; the socket fails with a permission
  error even though the user is in the group.
- Keep the container mounts in mind: the 86Box container sees only
  `$HOME`, `/tmp`, and the 86box-mcp repo. Put VM directories and every
  disk/floppy/CD image under one of those, or the guest sees nothing.
- Before starting any VM, check for strays: `docker ps` plus
  `ps | grep guard.py` (the guard's cmdline names its VM dir). A dead or
  stale MCP server leaves its container (`86box-<pid>`) running an old
  VM — two automation sockets exist and screen/key tools go to the
  machine you did not expect. Kill the stray guard; its `--rm` container
  cleans up.
- After any incremental CMake reconfigure of the 86Box build, verify
  `AUTOMATION`, `GDBSTUB`, and `QT` values in
  `.cache/build/CMakeCache.txt`. Silence is not success: the Qt6GuiPrivate
  `NOT FOUND` warning still configured, built (1036 targets), and linked.
- Treat `i386/debian:bullseye` as the last container base that carries
  `libtinfo5`/`libncurses5` for the i386 grub shell; keep its digest pinned
  (`assets/grub-legacy.toml`) and keep the fetched deb + extracted stages
  in the cache so the flow survives archive rot.

## Emulator discipline

- Keep held mode (server default): emulated time advances only inside
  tool calls. Boots become deterministic and interruptible;
  `step ms:1000` plus `read_memory` is the PIT timing instrument.
- Prefer `reset(kind:"hard")`. `kind:"cad"` (Ctrl+Alt+Del) was ignored
  once RosX owns the keyboard; hard reset always works. Boot-cycle
  evidence comes from hard resets only.
- Watch `vm_info.hard_resets`: any increment nobody typed is a triple
  fault, i.e. a finding. Record the counter at run start and end.
- After replacing a media file on the host, run `floppy_eject` +
  `floppy_insert` (or `cdrom_eject`/`cdrom_insert`). A running VM keeps
  reading through its old handle even across a guest reset, and the screen
  will look "stale" in the confusing direction.
- 86Box does not model QEMU's `-debugcon` 0xE9 port; writes there are
  simply harmless (which is itself the M5-5 check). Kernel-side serial
  logging, if ever wanted on era hardware, goes through the MCP
  `serial_read`/COM1 channel.

## Boot media

- Use etch-era GRUB Legacy 0.97-67, nothing newer: Debian's 2022
  `grub-legacy` build is not 486-clean (66 `cmov`/`cmpxchg8b` in
  `stage2_eltorito`). The offender floor per release: 0.97-64/-67: 0,
  0.97-70: 1, 0.97-72: 82, 0.97-80: 66. `cargo xtask check-isa x86_32
  --stages` enforces it; repointing the manifest at a drifted build must
  fail that gate.
- GRUB's `setup` demands `stage1`/`stage2` (and the needed `*_stage1_5`)
  *inside* the target filesystem — copy them to `/boot/grub` before
  running the grub shell, or `setup` fails.
- FAT12 boot floppies need no `stage1_5`: `setup (fd0)` runs in
  blocklist mode.
- Size partitions and FATs from the CHS count (993/16/63 → 1000944
  sectors), never from the image file size: 86Box's disk file is slightly
  larger than its CHS count.
- The [SiS 496] ASUS PVI-486SP3 BIOS has no El Torito (Boot Sequence C,A
  only), so the era CD path cannot boot there — that is a property of the
  board, not of the image: the same El Torito ISO and the stock GRUB-2 ISO
  both boot under QEMU `-cpu 486`. Era CD boot is Phase D territory
  (configs/machines.md).
- On the i386 grub shell never use `--privileged` and never need a loop
  device: `device (hd0) /path/to/image` maps a BIOS disk onto a plain
  file.

## Multiboot boundary observation

- The Multiboot magic `0x2BADB002` lives in `%eax`, not in RAM: scanning
  memory for it finds nothing. Read `%eax`/`%ebx` at an entry-point
  breakpoint (`breakpoint_set(addr=<kernel entry>, kind="exec")`; the
  entry prints on GRUB's handoff line, e.g. `entry=0x101000`).
- The info block sits in **low memory** (measured: `0x30748`, inside
  GRUB's own area) — not after the kernel image. The kernel's heap
  reclaims that region almost immediately after boot: post-boot memory
  scans for boot structures are a dead end. Stop the CPU at the entry
  instead.
- Prefer breakpoints over byte-pattern scans; where a scan is unavoidable,
  use exact 8-byte patterns — short patterns hit lookalikes in `.text`
  (measured: `02 b0ad 1b`).
- Synthesize bootloader variants at the entry stop: patching the FLAGS
  dword `0x3E7 → 0x00000001` with `write_memory` turns a stock mmap-carrying
  handoff into a sizes-only one — exactly the M5-1 fallback scenario —
  then `debug_continue` boots through it.
