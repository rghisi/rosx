# arch/x86-86box — era-hardware verification for x86_32

Data-and-scripts directory (no crate, no cargo membership): everything needed
to verify the x86_32 port on modeled 1990s hardware — a real Award BIOS, a
v1-only bootloader, an 8253/8259A on a SiS 496 chipset, an i486SX with no
FPU — driven headlessly through the [86box-mcp](https://github.com/dafugg/86box-mcp)
MCP server. Blueprint: `plans/x86-86box-integration-plan.md`; measured facts:
`plans/x86_32-i486-impl.md` (M5 supplement).

**Positioning:** 86Box evidence is *emulator evidence*. It never marks an
AGENTS.md hardware-checklist item green, and it never replaces the QEMU
suites — QEMU (with `check-isa`) stays the CI path; 86Box is the local
era-verification path and the hardware dry-run ground. CI cannot run 86Box
(licensed ROMs); CI guards mnemonics, the emulator proves boot.

## Layout

```
configs/     era VM template (verified byte-for-byte with the session) + board table
scripts/     fetch / era-media recipes / stage ISA audit — all unprivileged except the grub container
assets/      grub-legacy.toml: provenance manifest (deb URL + sha256, stage hashes, container digests, pins)
mcp/         opencode-86box.jsonc: MCP server registration snippet
docs/        gotchas.md (every trap is mandatory), evidence-checklist.md (M5 item → tool sequence)
```

## Prerequisites (host)

- Docker Engine, operator in group `docker` (`usermod -aG docker $USER`).
  **In agent shells wrap invocations as `sg docker -c '...'`** — an
  already-running session's process groups never pick up the new group.
- Unprivileged media tools: `sfdisk` (util-linux), `mtools`
  (`mformat`/`mmd`/`mcopy`/`mdir`), `xorriso`, `curl`, `dpkg-deb`,
  `objdump`. No loop devices anywhere in the flow.
- 86Box ROM set (the set the 86Box release ships, 477 machines) under
  `~/.local/share/86Box/roms`. **Licensed BIOS binaries — never commit
  them.** Manual download from the 86Box project; this path is what the
  MCP server config (`mcp/opencode-86box.jsonc`) expects.

## Build the emulator binary (one-time)

```sh
git clone https://github.com/dafugg/86box-mcp /tmp/opencode/86box-mcp
cd /tmp/opencode/86box-mcp
git clone https://github.com/86Box/86Box.git       # layout per repo docs
sg docker -c ./docker/build.sh                     # image "86box-mcp"
```

`docker/build.sh` configures `-D AUTOMATION=ON -D DISCORD=OFF -D MUNT=OFF
-D SOUNDCANVAS=OFF` → headless `86Box` with the automation socket. For the
Multiboot boundary probes (breakpoints, `%ebx` reads) also enable the GDB
stub — an incremental reconfigure of the same cache:

```sh
sg docker -c "docker run --rm -v $PWD:$PWD -w $PWD 86box-mcp \
  sh -c 'cmake -D GDBSTUB=ON .cache/build && cmake --build .cache/build'"
```

Verify `AUTOMATION`/`GDBSTUB`/`QT` in `.cache/build/CMakeCache.txt` after
any reconfigure — see `docs/gotchas.md`. Pinned commits and the expected
image id live in `assets/grub-legacy.toml`. **Mount discipline:** the
container sees only `$HOME`, `/tmp`, and the 86box-mcp repo — keep VM dirs
and all media under one of them.

## Register the MCP server (one-time)

Merge `mcp/opencode-86box.jsonc` into `~/.config/opencode/opencode.jsonc`
and restart OpenCode. The 86Box tools then arrive as `tools["86box"].*` in
Code Mode. Keep held mode (server default): emulated time advances only
inside tool calls, making boots deterministic and interruptible.

## Era media

```sh
cargo xtask build x86_32                     # kernel + apps + ROM
cargo xtask era-image x86_32                 # both artifacts below, one command
```

Or drive the scripts directly:

```sh
stages=$(arch/x86-86box/scripts/fetch-grub-legacy.sh | tail -1)
arch/x86-86box/scripts/build-legacy-iso.sh \
    target/rosx-i486/release/rosx-x86_32 "$stages" rosx-grub-legacy.iso
arch/x86-86box/scripts/build-legacy-hdd.sh \
    target/rosx-i486/release/rosx-x86_32 "$stages" rosx-hdd.img
```

- The **ISO** (El Torito, `stage2_eltorito`) passes a sizes-only MBI and is
  the `mem_upper`-fallback exercise. The 1996 Award BIOS on `486sp3c` has
  no El Torito — boot it with the QEMU control:
  `qemu-system-i386 -cpu 486 -m 8 -cdrom <iso> -boot order=d -debugcon stdio -no-reboot -display none`.
- The **HDD** (FAT16 + MBR stage1 + embedded `fat_stage1_5`) is the AGENTS.md
  hardware recipe executed literally — the era-authentic medium for this
  board class. Its grub `setup` runs in the pinned `i386/debian:bullseye`
  container (the i386 grub shell needs `libtinfo5`/`libncurses5`); no
  `--privileged`, no loop devices — `device (hd0) <file>` maps a BIOS disk
  onto a plain file.
- Both artifacts are guarded by hash-verified, pinned sources: GRUB Legacy
  **0.97-67** is the last 486-clean release (0.97-70 already drifts).
  `arch/x86-86box/scripts/check-isa-boot.sh` (or
  `cargo xtask check-isa x86_32 --stages`) re-audits the stages; CI runs it.

## VM bring-up (via MCP tools)

1. Clean strays first: `docker ps` + `ps | grep guard.py` (docs/gotchas.md).
2. `vm_create(vm_dir=<path under $HOME or /tmp>,
   config=<repo>/arch/x86-86box/configs/486sx-pvi486sp3.cfg)` — the server
   applies its primed CMOS NVR for `486sp3c`, so first boot needs no SETUP.
3. Copy the built `hdd.img` into the VM directory as `hdd.img` (the cfg's
   `hdd_01_fn`), or use `cdrom_insert`/`floppy_insert` for the other media.
4. `vm_start(held=true, binary=<86box-mcp>/docker/86box.sh)` — the binary
   must be the container launcher (docs/gotchas.md); with `debugger=true`
   the GDB stub joins for the boundary probes. `step ms:2000` → POST screen
   via `screen_text` (`80486SX CPU at 25MHz`, `Memory Test: 32768K OK`);
   `reset(kind:"hard")`, re-insert media, `wait_for_text("rose>", timeout_ms=180000)`.
   Complete measured runs: `target/86box/runs/<UTC>-<machine>/` (see
   `docs/evidence-checklist.md` for the full protocol).

Template keys (file kept comment-free, byte-identical to the verified
session): `time_sync=disabled` + `cpu_use_dynarec=0` keep runs
deterministic; `cpu_family=i486sx`/`fpu_type=none` is the no-FPU floor;
`hdd_01_speed=ramdisk` is host-side throughput only, it does not skip
drive timing; the 993/16/63 geometry is "big enough for the era" and
inside both BIOS translation limits.

## Bootloader era proof

The era bar for the BIOS-thunk bootloader is the fake-kernel boot of
`boot32.img`: `cargo xtask boot-image 32`, copied into the VM dir as
`hdd.img` and grown to 488 MiB, booted on era hardware. The screen must
render `[DISK]` (`media=chs` on a BIOS without EDD, `media=lba` on one
that serves it), `[ELF]`, `[INITRD]` and the fake kernel's
`initrd_crc32: 0x7551f9f4`, and the boot chain itself must add no
`vm_info` soft reset — real BIOS INT 13h/15h behavior (forced `sti`,
RAM-driver staging landmine, INT 15h IF-on-return, DL=0 hand-off) makes
this bar the era's strictest test of the thunk. Measured 2026-11-18,
three BIOS families, all PASS; Award and AMI take the same image bytes,
and the Phoenix board is why stage1 now carries a read-retry loop and a
boot-unit normalization (`plans/bootloader/14-era-thunk-stack.md` §6):

- **Award 4.51 — ASUS PVI-486SP3C (`486sp3c`, i486SX)**: the full-defect
  board — the three-era-defect chain and its instrumentation method are
  `plans/bootloader/14-era-thunk-stack.md` (run dir
  `target/86box/runs/20261118T172136Z-486sp3c/`).
- **AMIBIOS 4.01 AA.05.00 — HP Vectra 486VL (`vect486vl`, VLSI 82C480,
  i486SX)**: no-regression and fallback-path proof (run dir
  `target/86box/runs/20261118T183300Z-vect486vl/`). This BIOS refuses
  E820 *and* E801, so the boot exercises the `INT 88h` memory-detection
  fallback for the first time, and the IF-mask fix costs nothing on a
  BIOS that plays by the rules. CMOS note: bare defaults boot only the
  floppy — Setup's AUTO DETECT HARD DISK must accept the drive (Type 47,
  993/16/63) and WRITE TO CMOS AND EXIT once before the disk boots.
- **Phoenix 4.04 PnP — ECS UM8810P-AIO (`ecs486`, UMC 8881 PCI,
  i486SX)**: the first era board whose BIOS serves EDD (`media=lba`,
  closing the sweep's LBA gap), and the board that taught stage1 two
  era lessons: this BIOS hands the MBR `DL=0x00` (Award and AMI pass
  `0x80`), so every read — even the geometry probe, answered by the
  floppy BIOS — went to the configured-but-empty drive A as a
  motor-wait `AH=80h` timeout until an INT13-only probe MBR pinned it
  (`PROBE drv=00`), and the era `or dl,0x80` answer must be assembled
  per media type (`--defsym HARD_UNIT`) because the floppy checkpoint
  boots unit `0x00` legitimately. CMOS note: Setup → Fixed Disk Setup →
  IDE Adapter 0 Master → Autotype (Enter) accepts the drive once, then
  Save Values to CMOS; POST ends at "Incorrect Drive A type — Press
  F1" and F1 resumes into the MBR. `soft_resets: 1` here is the
  machine's power-on baseline — already counted on the first POST
  screen, before `[STAGE1]` — and the chain adds none (run dir
  `target/86box/runs/20261118T214900Z-ecs486/`).

## Production chain on era media (phase V)

Distinct bar from the `boot32.img` fake-kernel proof above: here the
**shipped artifact** — the unified disk (`target/rosx-i486-boot-disk.img`,
grown to the cfg geometry for the big-CMOS boards) with stage1 + stage2 +
`KERNEL.ELF` and the real release kernel — boots to `rose>`.

| board | media | verdict | run dir |
|---|---|---|---|
| Award 4.51 `486sp3c` | unified disk, 512 MB grown | `media=chs`, E820 accepted, `rose>` x3 | `target/86box/runs/20261121T152239Z-486sp3c-chain/` |
| AMI 4.01 `vect486vl` | unified disk, 512 MB grown | `media=chs`, **e820+e801 refused — INT 88h synthesis proven**; first boot exposed the VLSI phantom-slave defect (fixed in atad, see `docs/gotchas.md`), `rose>` x3 | `target/86box/runs/20261121T231200Z-vect486vl/` |
| Phoenix 4.04 PnP `ecs486` | unified disk, 512 MB grown | `media=lba` via EDD, **stage1 survives the `DL=0x00` hand-off**, e801 synthesis proven, `rose>` x4 | `target/86box/runs/20261121T233107Z-ecs486/` |
| Award 4.51 `486sp3c` | **UN-GROWN 64 MiB artifact** (131/16/63) | the shipped image bytes boot as-is: `media=chs`, mount ok, `rose>` x2 — the growth step above is a rehearsal artifact, not a boot requirement | `target/86box/runs/20261121T234310Z-486sp3c-ungrown/` |

With these three boards both memory-synthesis fallback branches (e801
and e88) and both media paths (chs and lba) of the production chain are
proven on era firmware. Bring-up notes: the AMI template against the
2026-11-18 8 MB primed NVR halts once with `CMOS memory size mismatch`
— SETUP -> STANDARD CMOS SETUP -> WRITE TO CMOS AND EXIT (Y) fixes it
permanently; the Phoenix template declares `fdd_01_type = 35_2hd`
because its CMOS expects a 1.44 MB drive A (`Incorrect Drive A type —
Press F1` without one). Templates: `configs/ami-486-vectra.cfg`,
`configs/phoenix-486-ecs.cfg`. Phoenix POST performs one soft reset
before the MBR runs — the chain adds none.

## Evidence run

Follow `docs/evidence-checklist.md` — it maps every hardware-checklist
item that has an emulator analog to its exact tool sequence, and defines
the run directory (`target/86box/runs/<UTC timestamp>-<machine>/`:
`screen.txt`, `screenshot.png`, `vm-info.json`, `mbi.hex`, `facts.md`).
`facts.md` opens with the sentence *emulator runs never flip a silicon
checkbox* by construction.

## Teardown

`vm_stop` every VM, then `docker ps` must show no stray `86box-*`
container. Evidence run dirs live under `target/` — archive them before
`cargo clean` if they should survive.
