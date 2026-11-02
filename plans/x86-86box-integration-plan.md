# 86Box Integration Plan — Era-Hardware Emulation for x86_32 Verification

> **Date:** 2026-11-02 · **Branch context:** `x86_32-i486-plan`
> **Status:** proposal — blueprint for structured adoption, following the
> exploratory session recorded in `plans/x86_32-i486-impl.md`
> ("M5 supplement — 86Box emulator session").
> **Companion evidence:** that section holds the measured facts; this file
> holds the reproduction blueprint and the adoption plan.

---

## 1. Why 86Box, and where it sits

QEMU's `-cpu 486` runner and `cargo xtask check-isa x86_32` guard the code
contract: forbidden mnemonics never ship. What neither can test is the
*firmware contract*: whether the port boots what a period machine actually
provides — a 1990s Award BIOS, a v1-only bootloader, an 8253/8259A on a SiS
496 chipset, a CPU with no FPU at all. 86Box models specific real machines
with their real BIOS images, so it closes the gap between "boots under a
generic multiboot host" and "boots on silicon".

Positioning, stated once so no later run confuses it:

- **86Box evidence is emulator evidence.** It never marks a hardware
  checklist item green. M5's seven silicon checks stay open regardless of
  what the emulator shows.
- 86Box complements, never replaces, the QEMU suites. QEMU stays the CI
  path (fast, deterministic, no ROM licensing); 86Box is the local
  era-verification path and the M5 dry-run ground.

What 86Box uniquely demonstrated in the exploratory session:

1. Boot to `rose>` on a modeled **i486SX-25 with no FPU** (`fpu_registers`
   returns `no FPU` while the kernel runs FP-free end to end).
2. Boot through the **era-authentic GRUB Legacy 0.97 recipe** from the
   AGENTS.md hardware instructions (MBR + `fat_stage1_5` + FAT16 `/boot`).
3. The Multiboot handoff observed at the kernel's **first instruction**
   (GDB-stub breakpoint), including a live read of the MBI the QEMU harness
   never exposed: GRUB Legacy 0.97-67 passes `FLAGS=0x3E7` **with a real
   e820 map**, and the `mem_upper` fallback needed only a one-dword patch
   at the breakpoint to be exercised to full boot.

## 2. Journey record — what was built and what it took

Reproducing the session means rebuilding four things: the host
prerequisites, the 86Box binary (in Docker), the MCP link into OpenCode,
and the era boot media. Each is recorded below as verified, plus the traps
found on the way.

### 2.1 Host prerequisites

- Docker Engine (`docker.io`), and the operator in group `docker`
  (`usermod -aG docker $USER`).
- **Trap:** an already-running agent session's process groups do not pick up
  the new group; `docker` fails with a socket permission error even after
  `usermod`. Wrap invocations: `sg docker -c '...'` (works passwordless
  once the usermod landed).
- Host tools for media building: `sfdisk` (util-linux), `mtools`
  (`mformat`/`mmd`/`mcopy`/`mdir`/`mtype`), `xorriso`, `objdump`,
  `mktemp`, `cpio`. All unprivileged — no loop devices are needed anywhere
  in the flow.
- 86Box ROM set (v6.0, 477 machines) installed under
  `~/.local/share/86Box/roms`. **Licensed BIOS binaries — never committed.**
  Manual download; the README states the source and expected location.

### 2.2 The 86Box build (Docker path)

Via <https://github.com/dafugg/86box-mcp> ("Or build and run 86Box in
Docker"), cloned to a scratch dir (`/tmp/opencode/86box-mcp`), with 86Box
master inside it:

```sh
cd /tmp/opencode/86box-mcp
git clone https://github.com/86Box/86Box.git    # layout per repo docs
sg docker -c ./docker/build.sh                  # image "86box-mcp", build cache .cache/build
```

`docker/build.sh` configures with `-D AUTOMATION=ON -D DISCORD=OFF
-D MUNT=OFF -D SOUNDCANVAS=OFF` and produces `.cache/build/src/86Box`,
launched by `docker/86box.sh` with `QT_QPA_PLATFORM=offscreen` — a headless
binary exposing the automation socket the MCP server drives.

For the boundary probes (breakpoints/watchpoints, `%ebx` MBI reads) the
binary additionally needs the GDB stub — an incremental reconfigure of the
same cache:

```sh
sg docker -c "docker run --rm -v $PWD:$PWD -w $PWD 86box-mcp \
  sh -c 'cmake -D GDBSTUB=ON .cache/build && cmake --build .cache/build'"
```

Result: `vm_start(debugger=true)` reports `debugger on port <n>`.
**Trap:** the reconfigure printed a `Qt6GuiPrivate ... NOT FOUND` warning
yet configured and rebuilt (1036 targets) and linked fine; verify
`AUTOMATION`/`GDBSTUB`/`QT` values in `.cache/build/CMakeCache.txt` after
any reconfigure rather than trusting silence.

**Mount discipline:** the container mounts only `$HOME`, `/tmp`, and the
86box-mcp repo. VM directories and all media (disk/floppy/CD images) must
live under one of them.

### 2.3 MCP integration (OpenCode as the driver)

Register 86Box as a native MCP server (global
`~/.config/opencode/opencode.jsonc`) — this is what makes all 72 tools
first-class for an agent, and is strictly preferable to a side CLI:

```jsonc
"mcp": {
  "servers": {
    "86box": {
      "type": "local",
      "command": ["sg", "docker", "-c", "/tmp/opencode/86box-mcp/server/.venv/bin/box86mcp"],
      "environment": {
        "BOX86_ROMS": "/home/ghisi/.local/share/86Box/roms",
        "BOX86_WINDOW": "0"
      }
    }
  }
}
```

Tools are then reached through Code Mode as `tools["86box"].*`. The session
used: `vm_create` (from a template `86box.cfg`; applies the repo's primed
CMOS NVR for that machine), `vm_start` (`held` default; `debugger` for the
GDB stub), `step`, `wait_for_text`, `screen_text`, `press_keys`,
`type_text`, `reset` (`hard`/`cad`), `floppy_insert`/`floppy_eject`,
`cdrom_insert`, `read_memory`, `write_memory`, `find_memory`,
`breakpoint_set`, `run_until_break`, `debug_continue`, `registers`,
`fpu_registers`, `vm_info`, `vm_status`, `vm_stop`, `screenshot`.

**Held-mode discipline** (server default, keep it): emulated time advances
only inside tool calls. Boots become deterministic and interruptible;
`step ms:1000` + memory reads are the PIT timing instrument (§3 of the
evidence log).

**Operational traps (all hit once in the session):**

- One guard process + one container per started VM, named `86box-<pid>`. A
  dead/stale MCP server can leave a container running the old VM — two VMs,
  two automation sockets, and screens that look "stale" because reads and
  keys go to a different machine than expected. Diagnose with `docker ps`
  + `ps | grep guard.py` (the guard's cmdline names its VM dir); kill the
  stray guard and its `--rm` container cleans up.
- Media hot-swap: after replacing an image file on the host, use
  `floppy_eject`/`floppy_insert`; a running VM can keep reading through its
  old handle even across a guest reset.
- `reset(kind:"cad")` was ignored once RosX owned the keyboard; `kind:"hard"`
  always works. Prefer hard for boot-cycle evidence.
- `vm_info.hard_resets` is the triple-fault tripwire: any increment nobody
  typed is a finding.

### 2.4 The machine: `[SiS 496] ASUS PVI-486SP3` (`486sp3c`)

Selection criteria: accepts `cpu_family=i486sx`; on-board IDE (the repo
recipe targets IDE); RAM covers the 8 MB era board and beyond; BIOS of the
SX era; **and** — the tiebreaker that made bring-up one step — 86box-mcp
ships a primed CMOS NVR for it, so first boot needs no SETUP session.
POST confirmed `80486SX CPU at 25MHz`, `Memory Test: 8192K OK`; SETUP
showed `Co-Processor: Not Installed`, `Extended Memory: 7168K`.

Verified template (`configs/486sx-pvi486sp3.cfg` in §4):

```ini
[General]
confirm_exit = 0
confirm_reset = 0
confirm_save = 0
vid_renderer = qt_software

[Machine]
time_sync = disabled
cpu_family = i486sx
cpu_multi = 1
cpu_speed = 25000000
cpu_use_dynarec = 0
fpu_type = none
machine = 486sp3c
mem_size = 8192

[Video]
gfxcard = s3_trio64_pci

[Input devices]
keyboard_type = keyboard_ps2
mouse_type = ps2

[Hard disks]
hdd_01_fn = hdd.img
hdd_01_ide_channel = 0:0
hdd_01_parameters = 63, 16, 993, 0, ide
hdd_01_speed = ramdisk

[Floppy and CD-ROM drives]
fdd_01_type = 35_2hd
cdrom_01_parameters = 1, atapi
cdrom_01_ide_channel = 0:1
```

`time_sync=disabled` and `cpu_use_dynarec=0` keep runs deterministic;
`hdd_01_speed = ramdisk` is host-side throughput only and does not skip
drive timing. The disk the VM creates for that geometry is 993/16/63
(488 MB) — "big enough for the era" and inside both BIOS translation limits.

**BIOS-era finding:** this Award BIOS (06/25/96) offers `Boot Sequence`
C,A **only** — no El Torito. Combined with the live finding that GRUB
Legacy 0.97 cannot address the ATAPI CD as a disk device
(`root (cd)` → `Error 23`; 0.97 reads ISOs only through `stage2_eltorito`'s
own boot-info table, which needs the El Torito boot the BIOS lacks), the
era conclusion is settled: **on a board of this vintage the HDD is the
authentic RosX boot medium**, and the AGENTS.md recipe is it. (The CD wish
is still reachable on later-486 boards — Phase D.)

### 2.5 Era boot media — the three recipes, all unprivileged

**Bootloader choice.** Debian's modern `grub-legacy` (2022) build is **not
486-clean** — 66 `cmov`/`cmpxchg8b` instructions in `stage2_eltorito`.
Measured drift (five-86 offender counts per stage set):
`0.97-64/-67: 0 · 0.97-70: 1 · 0.97-72: 82 · 0.97-80: 66`.
The floor is **etch-era `grub-legacy 0.97-67`** from archive.debian.org:
0 offenders, and proven to boot RosX under QEMU `-cpu 486`.

**The grub shell** (`/usr/sbin/grub` in the deb) is an i386 ELF; on an
x86_64 host run it in a container. Debian trixie i386 no longer ships
`libtinfo5`/`libncurses5`, so the working image is **`i386/debian:bullseye`**
(install `libtinfo5 libncurses5`, `dpkg -i` the deb). No `--privileged`
flag, no loop devices: GRUB's `device (hd0) /path/to/image` maps a BIOS
disk onto a plain file.

Recipe 1 — **GRUB Legacy El Torito CD** (`stage2_eltorito` passes a
sizes-only MBI, so QEMU boots through it exercise the `mem_upper` fallback):

```sh
work=$(mktemp -d); mkdir -p "$work/boot/grub"
cp "$stages/stage2_eltorito" "$work/boot/grub/"
cp target/rosx-i486/release/rosx-x86_32 "$work/boot/rosx"
printf 'timeout 0\ntitle RosX x86_32 on 486SX via GRUB Legacy\nkernel /boot/rosx\n' \
    > "$work/boot/grub/menu.lst"
xorriso -as mkisofs -r -V ROSX_LEGACY \
    -b boot/grub/stage2_eltorito -no-emul-boot -boot-load-size 4 -boot-info-table \
    -o rosx-grub-legacy.iso "$work"
```

Control under QEMU: `qemu-system-i386 -cpu 486 -m 8 -cdrom <iso>
-boot order=d -debugcon stdio -no-reboot -display none` → boots; the stock
GRUB-2 ISO of the same kernel also boots there (it is the CD-boot-hostile
BIOS on `486sp3c` that refuses it, not the image).

Recipe 2 — **GRUB Legacy FAT16 HDD** (the AGENTS.md hardware recipe,
executed literally; this is the image that booted the 486SX to `rose>`):

```sh
# partition: one bootable FAT16 starting at 63 (gap after MBR holds stage1_5)
printf 'start=63, size=1000266, type=6, bootable\n' | sfdisk --no-reread hdd.img
# FAT16 at partition offset 63*512=32256, geometry matching the disk
mformat -i hdd.img@@32256 -h 16 -s 63 -T 1000266 -F -v ROSX
mmd -i hdd.img@@32256 ::/boot ::/boot/grub
for f in stage1 stage2 fat_stage1_5; do
  mcopy -m -o -i hdd.img@@32256 "$stages/$f" ::/boot/grub/; done
mcopy -m -o -i hdd.img@@32256 target/rosx-i486/release/rosx-x86_32 ::/boot/rosx
printf 'timeout 5\ntitle RosX x86_32 on emulated 486SX-25 (GRUB Legacy 0.97-67 on HDD)\nkernel /boot/rosx\n' \
    > menu.lst && mcopy -m -o -i hdd.img@@32256 menu.lst ::/boot/grub/menu.lst
```

```sh
# stage1 -> MBR, fat_stage1_5 -> the gap, prefix wired to (hd0,0)/boot/grub
sg docker -c "docker run --rm -v /tmp:/tmp i386/debian:bullseye bash -c '
  apt-get update -qq && apt-get install -y -qq libtinfo5 libncurses5
  dpkg -i /tmp/.../grub-legacy_0.97-67_i386.deb
  grub --batch --no-floppy --device-map=/dev/null <<EOF
device (hd0) /tmp/.../hdd.img
root (hd0,0)
setup (hd0)
EOF'"
```

**Trap:** `setup` demands `stage1`/`stage2` (and the needed `*_stage1_5`)
*inside* the target filesystem first — copy them there before running it.

Recipe 3 — **GRUB boot floppy** (`setup (fd0)`, blocklist mode — FAT12
floppies need no stage1_5). The BIOS falls through blank-C: to A:, so the
floppy chain also booted the 486SX and stage2 reached its command line —
this is how the `root (cd)` Error-23 finding was measured live.

### 2.6 Boundary observation pattern (the GDB-stub flow)

The reusable technique for any future "what did the bootloader actually
pass" question:

1. `vm_start(debugger=true, held=true)`.
2. `breakpoint_set(addr=<kernel entry>, kind="exec")` — the entry is
   printed by GRUB's handoff line (`entry=0x101000`), or read from the
   ELF.
3. Boot; `run_until_break` stops the CPU at the kernel's first
   instruction. `%eax` must be `0x2BADB002`; `%ebx` is the MBI pointer.
4. `read_memory` at `%ebx`: decode FLAGS and the fields of interest;
   `write_memory` can *synthesize* bootloader variants (the M5 fallback
   check is exactly a FLAGS dword patch — `0x3E7 → 0x00000001` — at this
   stop, then `debug_continue` to full boot).

Measured on this stack: GRUB Legacy 0.97-67 disk boot passes
`FLAGS=0x3E7` — mmap bit **set**, five e820 entries at `0x572e8`, the
extended entry `{0x100000, 0x700000, usable}` agreeing with
`mem_lower=0x27F`/`mem_upper=0x1C00`. A stock 0.97 handoff therefore takes
the same `mmap` branch as GRUB 2; the sizes-only branch is what
`stage2_eltorito` and the patched-FLAGS experiment exercise.

**Traps found here, worth passing on:**

- The MBI carries **no magic in RAM** — the magic lives in `%eax`. Scanning
  memory for `0x2BADB002` finds nothing; the MBI sits in **low memory**
  (`0x30748` — inside GRUB's own area), not after the kernel image.
- The kernel's heap reclaims the MBI region almost immediately after boot;
  post-boot memory scans for boot structures are a dead end. Stop the CPU
  at the entry instead.
- Byte-pattern scans over the kernel image hit lookalikes (`02 b0ad 1b` in
  `.text`); prefer exact-8-byte patterns or, better, breakpoints.

Boot-media matrix, settled by evidence:

| Boot path | MBI passed | Kernel branch | On `486sp3c` | On QEMU `-cpu 486` |
|---|---|---|---|---|
| GRUB 2 ISO (xtask image) | mmap | `mmap_memory` | ✗ BIOS cannot CD-boot | ✓ |
| GRUB Legacy 0.97-67 HDD | `0x3E7`, mmap+sizes | `mmap_memory` | ✓ to `rose>` | ✓ |
| same, FLAGS→`0x1` at breakpoint | sizes only | `sized_memory` | ✓ to `rose>` | n/a |
| GRUB Legacy `stage2_eltorito` ISO | sizes only (bit0) | `sized_memory` | ✗ BIOS cannot CD-boot | ✓ |

## 3. Boot process and image generation — assessment

**The kernel needs no change.** Both decode branches of
`arch/x86-generic/src/multiboot.rs` are now proven on a modeled era machine:
mmap as passed by stock GRUB Legacy and GRUB 2, and the `mem_upper`
fallback with mmap suppressed at the true handoff point. Every required
change lands in *image generation and tooling*:

1. **Era image recipe in xtask.** Add
   `cargo xtask era-image x86_32` producing both era artifacts:
   `rosx-x86_32-legacy.iso` (Recipe 1) and `rosx-x86_32-legacy-hdd.img`
   (Recipe 2, including the containerized grub-shell `setup`). The stock
   `cargo xtask image x86_32` GRUB-2 ISO stays exactly as it is.
   Host requirements are the unprivileged set (§2.1) plus docker for the
   grub shell; xtask detects and reports each missing tool by name.
2. **Stage provenance manifest.** `arch/x86-86box/assets/grub-legacy.toml`
   pins the archive.debian.org deb URL, its sha256, and the sha256 of each
   extracted stage file. The build fetches, verifies, and caches under
   `target/`. Nothing binary enters git; the hashes are the regression
   guard against a silent stage swap (the 0.97-70..80 drift table is the
   reason this matters).
3. **check-isa extends to the bootloader.** A `--stages` mode (or
   `check-isa-boot.sh` standalone) runs the existing 486 mnemonic audit
   over `stage2`, `stage2_eltorito`, `fat_stage1_5` — the audit already
   exists in xtask and works on stripped binaries via `objdump -d`. CI can
   run this from the manifest hash alone, no docker, no ROMs.
4. **AGENTS.md updates** once the above land: xtask command table, the
   `arch/x86-86box` line in the structure map, and one hardware-recipe
   sentence noting the era path is now scripted (`--legacy-hdd` equivalent)
   rather than manual.

## 4. `arch/x86-86box` — the adoption vehicle

A data-and-scripts directory (no crate, no cargo membership — workspace
builds are untouched), holding everything needed to reproduce this
verification without tribal knowledge:

```
arch/x86-86box/
  README.md                      # the runbook: prerequisites, build,
                                 #   MCP registration, VM bring-up, evidence
                                 #   recipe, teardown — one page, verbatim-verified commands
  configs/
    486sx-pvi486sp3.cfg          # era baseline VM template (§2.4), commented
    machines.md                  # candidate-board table (§2.4 criteria +
                                 #   BIOS era capabilities, Phase D rows)
  scripts/
    fetch-grub-legacy.sh         # download + sha256-verify 0.97-67 deb
                                 #   (arch/x86-86box/assets/grub-legacy.toml)
    build-legacy-iso.sh          # Recipe 1 (§2.5), params: kernel ELF, output ISO
    build-legacy-hdd.sh          # Recipe 2 (§2.5): sfdisk + mformat + mtools +
                                 #   containerized grub shell; params: kernel ELF,
                                 #   disk size, output image
    check-isa-boot.sh            # mnemonic audit of GRUB stage binaries
  assets/
    grub-legacy.toml             # provenance manifest (URLs + sha256, §3.2)
  mcp/
    opencode-86box.jsonc         # drop-in mcp.servers.86box snippet (§2.3)
  docs/
    gotchas.md                   # the trap lists from §2, in imperative form
    evidence-checklist.md        # M5 item → exact MCP tool sequence that
                                 #   produces its emulator-side analog
```

Licensing decisions encoded in the layout: **no BIOS ROMs in the tree**
(copyright; README names the 86Box ROM package and the install path); GRUB
Legacy stages are fetched and hash-verified, not vendored; the container
images referenced by the scripts (`86box-mcp` built from the pinned
86box-mcp commit, `i386/debian:bullseye`) are named with digests in
`assets/grub-legacy.toml` so rebuilds are reproducible.

## 5. Evidence pipeline (what a run must leave behind)

One directory per run, `target/86box/runs/<UTC timestamp>-<machine>/`:

- `screen.txt` — `screen_text` captures at each milestone (POST,
  bootloader banner, kernel banner, `rose>`).
- `screenshot.png` — `screenshot` at `rose>`.
- `vm-info.json` — `vm_info` (machine, cpu, `hard_resets`).
- `mbi.hex` — first-instruction MBI dump via the §2.6 pattern.
- `facts.md` — the observation table, keyed to the M5 checklist:
  POST reports SX/8 MB · no-FPU kernel run · PIT 1:1 clock advance
  (`read_memory` of `SYSTEM_TIME_MS`, `step ms:1000`, delta) · ATAD
  identify + LBA read (`ata` app: `55AA ok`) · heap probe (`mem` app) ·
  boot media matrix row exercised and MBI FLAGS.
- Teardown rule: `vm_stop` every VM, `docker ps` shows no stray
  `86box-*` container; run dir survives `cargo clean` if archived.

Emulator runs never flip a silicon checkbox; `facts.md` opens with that
sentence by construction.

## 6. Phased adoption

**Phase A — scaffold (`arch/x86-86box` lands).**
Copy the verified §2 recipes into `scripts/` (parameterized: repo root,
kernel path, stage dir, output), write README + gotchas from §2's trap
lists, add the manifest with the 0.97-67 hashes measured this session.
*Acceptance:* from a clean checkout, `fetch-grub-legacy.sh &&
build-legacy-hdd.sh && build-legacy-iso.sh` reproduce byte-equivalent
stage wiring (bootable artifacts; MBR/blocklists deterministic from the
same inputs), and a follow-on 86Box run reproduces the `rose>` boot.

**Phase B — xtask integration.**
`cargo xtask era-image x86_32` shells into the Phase-A scripts;
`check-isa` gains `--stages`; CI (`build-x86_32`) runs the stage audit
from the manifest hash. AGENTS.md updated per §3.4.
*Acceptance:* xtask builds both era artifacts in one command; the stage
audit fails CI if the manifest is repointed to a post-drift GRUB build;
no change to existing `image`/`build`/`run` behavior.

**Phase C — evidence standardization.**
`evidence-checklist.md` maps every M5 item to its emulator analog and the
exact tool sequence (the session's flow becomes the template). A run per
release candidate, archived `facts.md` referenced from the impl log.
*Acceptance:* one complete run dir per M5 item that has an emulator
analog, linked from `plans/x86_32-i486-impl.md`.

**Phase D — era CD boot on a later-486 board (optional).**
The CD wish survives as: late-486 PCI boards whose Award/AMI BIOSes gained
`Boot Device: ATAPI CD-ROM` (candidates already vetted as
i486SX-accepting + IDE: Zida Tomato **4DPS**, **Rise R418**, **FIC
486-VIP-IO2**, all SiS 496/VIA-chipset 1997-98 designs). These machines
have no primed NVR in 86box-mcp, so the flow adds one SETUP automation
(F1 at checksum-error halt → set boot order → F10), after which the saved
NVR makes it as reproducible as `486sp3c`. If one boots the
`stage2_eltorito` ISO, the sizes-only path gets verified *without* a FLAGS
patch, and `machines.md` gains the board with its BIOS-capability row.
*Acceptance:* `rose>` reached from CD on one listed board, documented as
emulator evidence with the §2.6 FLAGS capture.

## 7. Risks and open questions

- **ROM licensing** — manual download, never in git; README carries the
  obligation. CI cannot run 86Box; emulator evidence stays a local/attested
  artifact. (Decision encoded: CI guards mnemonics, emulator proves boot.)
- **Third-party pins** — 86box-mcp and 86Box master drift; pin both commit
  SHAs in `assets/`, upgrade deliberately, re-run the §2.6 capture on
  upgrade.
- **Container aging** — `i386/debian:bullseye` is the last image with the
  libs the grub shell deb needs; pin its digest, and keep the deb +
  extracted stages in the fetch cache so the flow survives archive rot.
- **Reconfigure integrity** — verify `CMakeCache.txt` flags after every
  incremental rebuild (§2.2 Qt6-warning anecdote is the cautionary tale).
- **`sg docker` dependency** — agent shells lack the group; document the
  wrapper rather than "fix the session".
- **`cad` under a running kernel** — Ctrl+Alt+Del did not reboot through
  RosX's keyboard stack. Era boards reset at hardware level; investigate
  whether RosX's ps2d should claim `CTRL_ALT_DEL` as a warm-reboot affordance
  (open question, not required by any phase).
- **No `-debugcon` analog** — 86Box does not model the 0xE9 debug port; the
  MCP `serial_read`/COM1 channel exists if kernel-side serial logging is
  ever wanted on era hardware. VGA text (`screen_text`) suffices today.
- **Geometry slack** — 86Box's disk file is slightly larger than its
  993/16/63 CHS count; partition/FAT sizing must use the CHS count (as the
  verified recipe does), not file size.

## 8. Non-goals

- No 86Box execution in CI.
- No changes to `kernel/`, `usrlib/`, apps, or the boot protocol.
- No vendoring of ROMs or GRUB binaries.
- No replacement of QEMU suites; `tests-integration` stays the gate.
- No hardware checklist changes — the seven M5 items close on silicon only.
