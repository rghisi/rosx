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

## Emulator discipline

- `vm_start` needs `binary=/tmp/opencode/86box-mcp/docker/86box.sh` (the
  container launcher) on the Docker build path: the server's default binary
  resolution points at the container-built ELF and execs it on the *host*,
  which lacks `libopenal` ("error while loading shared libraries"). The
  wrapper runs the same ELF inside the `86box-mcp` image and keeps VM/ROM/
  socket paths identical (`$HOME`, `/tmp`, repo are mounted at host paths).
- Keep held mode (server default): emulated time advances only inside
  tool calls. Boots become deterministic and interruptible;
  `step ms:1000` plus `read_memory` is the PIT timing instrument.
- A `breakpoint_set` address survives `reset(kind:"hard")`: re-setting the
  same entry fails with `GdbError … 'E22'` (duplicate) — harmless, the old
  breakpoint fires on the next boot; just `run_until_break` again.
- `screenshot` returns the frame to the agent inline, never a file. For the
  evidence directory's `screenshot.png`: `record_start(directory=…, sound:false)`
  → `step ms:600` → `record_stop`, then copy one frame PNG. (As of 2026-11-21
  `record_start` raises a NumPy C-extension ImportError in the 86box-mcp
  server venv; until that env is fixed, the inline `screenshot` stands as the
  run's visual and `screen.txt` is the primary artifact.)
- In the production-chain real-kernel boot, stage2's ELF-load window is ~3–4 s
  of emulated time after the `[DISK]` banner, then `[ELF]` through `[BOOTINFO]`
  all print inside ~150 ms — a 1000 ms capture cadence scrolls the entire
  handoff away between two frames. Poll until `[DISK]` appears, then step 100 ms.
- `fpu_registers` reports `no FPU` by **raising** — on an i486SX floor machine
  that error is the pass result. Run it alone, last: the raise aborts any
  combined probe call that shares it.
- `write_memory` takes the bytes as `hex_bytes` (a string like
  `"01 00 00 00"`); `data`/`value`-style arguments fail with
  "give hex_bytes or value".
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

- The era boot medium is the production unified disk
  (`target/rosx-i486-boot-disk.img`), written whole to the IDE image file
  the cfg names. No external bootloader, no container: stage1 + stage2 are
  built and ISA-audited by `cargo xtask build x86_32` itself.
- Size partitions and FATs from the CHS count (993/16/63 → 1000944
  sectors), never from the image file size: 86Box's disk file is slightly
  larger than its CHS count.
- mtools `@@` offsets are BYTES, not sectors: the unified boot disk's FAT
  slice at LBA 2048 is inspected with `mdir -i disk.img@@1048576 ::`;
  `@@2048` answers "non DOS media" and reads like a corrupt image.
- The [SiS 496] ASUS PVI-486SP3 BIOS has no El Torito (Boot Sequence C,A
  only), so no CD path exists on that board. The production chain is
  MBR-booted anyway; era CD boot (phase V4) stays an open exploration,
  not a boot requirement (configs/machines.md).
- The VLSI 82C480 (vect486vl) does NOT float an absent IDE slave: status
  reads `0x00`, not `0xFF`, so the floating-bus guard never fires and the
  phantom passes the signature check. Only the DrivePresent rule (`!BSY
  && !DRDY` = nothing attached) declares it absent fast — without it the
  probe spins its DRQ budget past init's 10 s service wait and the boot
  falls back to the ROM shell (measured 2026-11-21, run
  `20261121T231200Z-vect486vl`; the fix lives in `apps/atad`).

## Boot-handoff observation

- Where a breakpoint probe is needed (e.g. watching the stage2 -> kernel
  handoff), prefer breakpoints over byte-pattern scans: short patterns hit
  lookalikes in `.text` (measured during the GRUB era: `02 b0ad 1b`); use
  exact 8-byte patterns if a scan is unavoidable.
- Post-boot memory scans for boot structures are a dead end: the kernel's
  heap reclaims low boot areas almost immediately. Stop the CPU at the
  entry instead.
