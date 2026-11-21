# x86_32 era verification and GRUB Legacy retirement — plan for later execution

Status: **phase V executed 2026-11-21** (V1/V2/V3 green, V4 not run —
optional, does not gate R). V1: AMI `vect486vl`
(`target/86box/runs/20261121T231200Z-vect486vl/`) — e820+e801 refused,
INT 88h synthesis proven; first boot exposed the VLSI phantom-slave
defect, fixed in atad (DRDY presence rule, commit `2c2cf95`). V2:
Phoenix `ecs486` (`target/86box/runs/20261121T233107Z-ecs486/`) —
`DL=0x00` survived, `media=lba`, e801 synthesis proven. V3: the
un-grown 64 MiB artifact boots as built on Award
(`target/86box/runs/20261121T234310Z-486sp3c-ungrown/`). The growth
step is confirmed a rehearsal artifact. The earlier groundwork
rehearsal evidence lives in
`target/86box/runs/20261121T152239Z-486sp3c-chain/`. Phase R remains
executed only when the trigger fires; with V1+V2 green it is honest
(AGENTS.md silicon checklist still open).

## 1. Context

The carrier swap (`plans/x86_32-carrier-swap.md`, head `c5e4ee2`, PR #188 green)
made the custom stage1→stage2→BootInfo chain the x86_32 boot path for QEMU, CI
and the harness. The modern-GRUB footprint is gone (M4: 191 deletions). What
survives is the **GRUB Legacy 0.97 era media** stack (`arch/x86-86box/` era
scripts, `cargo xtask era-image`, CI stage audit) plus the three kernel/arch
pieces era media depends on (Multiboot v1 header, `0x2BADB002` branch,
`arch/x86-generic/src/multiboot.rs`). Their removal is the C2 sweep, gated by
the trigger in `docs/todo/multiboot-retirement.md`.

Two boot-story layers must not be conflated (this tripped the rehearsal readout
once, so state it plainly):

- **Chain raw-load bar (2026-11-18, proven on 3 BIOS families):** `boot32.img`
  (fake kernel, raw-sector load) boots stage1+stage2 on Award 4.51 (`486sp3c`),
  AMI 4.01 (`vect486vl`) and Phoenix 4.04 (`ecs486`).
- **Production bar (2026-11-21 rehearsal, proven on 1 family):** the shipped
  unified disk — FAT slice + `KERNEL.ELF` + BootInfo + real release kernel to
  `rose>` — boots on the **Award** board only (plus QEMU). AMI and Phoenix have
  never seen the FAT slice, the E820/synthesis map builders, or BootInfo.

## 2. Rehearsal results (2026-11-21, emulator evidence only)

Machine `486sp3c` (ASUS PVI-486SP3, Award Modular v4.51PG 06/25/96, i486SX-25,
no FPU, 32 MB, held time); media = `target/rosx-i486-boot-disk.img` grown to
`512483328` B to match the cfg geometry (993/16/63). Three boots to `rose>`,
no spurious resets (`hard_resets` delta == the two intentional hard resets).

| observation | value | why it matters |
|---|---|---|
| stage1 on real Award | `[STAGE1] OK` | MBR chain load from a 1996 BIOS |
| BIOS disk view | `Mode 3, 512MB`; `[INT13] 992c 16h 63s` | translated geometry |
| media branch | **`media=chs`** | the CHS fallback — never taken in CI (QEMU gives `lba`) — works end to end with the real kernel |
| stage2 memory | E820 **accepted** (no `[MEM] e820 refused`), 9 entries, `gap=0xc2000 OK` | E820 builder proven on era firmware; the e801/e88 **synthesis fallback is still unproven** on any board |
| handoff | `[STAGE2_32] A20=OK THUNK=OK`; `[BOOTINFO] regions=9 ... dropped=0`; kernel `[MEMORY] Total: 30 MB` | A20 + real-mode thunk + BootInfo on the Award full-defect board |
| kernel + apps | 14-step plan, rxfs `ata0d0p0` mounted `/`, `ls`/`ata`/`mem` healthy | real 768 KiB kernel from disk to shell |
| PIT (item 3 analog) | `SYSTEM_TIME_MS` +1000 over `step ms:1000` | 8253 timing exact |
| ATA (item 4 analog) | `mbr signature 55AA ok`, `part0 boot=80 type=06 lba=2048 sectors=129024` | the `ata` app read the **chain's own MBR** via 16-bit PIO |
| no-FPU (item 7) | `fpu_registers` → `no FPU`, FP-free boot | i486SX floor holds for the chain |

Verdict: **the production chain is era-plausible on Award**; nothing found that
would block retirement of era media on this family. Branches still unproven on
era firmware: E820-refusal synthesis (needs AMI `vect486vl`: refuses E820 *and*
E801 → INT 88h path), LBA media + `DL=0x00` boot-drive quirk (needs Phoenix
`ecs486`), and any CD/El Torito story (N/A on Award; Phase D boards per
`configs/machines.md`). Silicon items stay open regardless — emulator runs never
flip them (AGENTS.md).

## 3. Phase V — era verification of the production chain (prerequisite work)

Each V step produces a run dir `target/86box/runs/<UTC>-<machine>/` with
`screen.txt`, `vm-info.json`, `facts.md` per `arch/x86-86box/docs/evidence-checklist.md`
(the checklist's item mapping already fits; `mbi.hex` is chain-replaced by the
`[BOOTINFO]` banner line in `screen.txt`).

- **V1 — AMI board (`vect486vl`, AMI 4.01 AA.05.00, HP Vectra 486VL).**
  Create `configs/ami-486-vectra.cfg` from the 2026-11-18 exploratory session's
  key set (only `486sx-pvi486sp3.cfg` is committed today). Boot the unified disk
  (grown as in the rehearsal) to `rose>` ×2. **Must-see:** `[MEM] e820 refused,
  trying e801` and the synthesized map banner, `[STAGE2_32] A20=OK THUNK=OK`,
  plausible `[MEMORY] Total`, mount + shell. Pass = the synthesis fallback is
  proven on era firmware for the first time (stronger than the GRUB-era story,
  which never exercised it end-to-end to shell with the real kernel).
- **V2 — Phoenix board (`ecs486`, Phoenix 4.04 PnP, ECS UM8810P-AIO).**
  Create `configs/phoenix-486-ecs.cfg` likewise. **Must-see:** stage1 survives
  `DL=0x00` (drive-byte hardening from `plans/bootloader/14-era-thunk-stack.md`),
  which media string it picks (`lba` expected via EDD), read-retry behavior, and
  full boot to `rose>` ×2.
- **V3 — geometry case on the Award board.** Boot the **un-grown** 64 MiB image
  once (cfg with matching small geometry, or 86Box's own disk sizing) to show the
  production artifact itself — not a grown copy — boots on a period BIOS; the
  slice needs ~130 cylinders, far inside the 1024-cylinder CHS wall.
- **V4 (optional, does not gate R) — Phase D El Torito boards** (`4dps` et al.):
  CD boot was always a GRUB-era *wish*; the chain is HDD/MBR-based and retirement
  does not depend on it. Keep as independent era-CD work.

Gate for phase V: the README's era section gains a production-chain row per
board; every claim carries a run dir; AGENTS.md silicon checklist untouched.

## 4. Phase R — the C2 sweep (executed only when the trigger fires)

Trigger (from `docs/todo/multiboot-retirement.md`): era media leave the x86_32
support matrix. That is a **maintainer decision** — V1+V2 green makes it honest;
a maintainer may also fire it early on the rehearsal evidence alone, recorded as
a decision in `docs/decisions/`. R is one reviewable commit series; gate
(`cargo xtask check`) green per commit, full `cargo xtask verify` at the end.

- **R1 — tag + decision record.** Annotated tag (`last-multiboot-kernel`) at the
  sweep's parent for future era experiments, plus `docs/decisions/` entry:
  verdict summary, links to run dirs, unsafe/ISA baseline effects.
- **R2 — era-media tooling deletion:**
  `arch/x86-86box/scripts/{build-legacy-hdd,build-legacy-iso,fetch-grub-legacy,check-isa-boot}.sh`,
  `assets/grub-legacy.toml`; xtask `era-image` subcommand, `require_era_tools`,
  the stages-directory default, the era hint text; `check_isa.rs` `--stages`
  audit path; `ci.yml` era steps (stage fetch + boot-stage audit).
  `cargo xtask boot-image 32|64` (our fake-kernel chain images) **stays** — it is
  chain tooling, not GRUB tooling.
- **R3 — kernel/arch deletion:** the `.multiboot` header section in
  `arch/x86_32/src/boot.S`; the `0x2BADB002` branch in `kernel_main`
  (`arch/x86_32/src/main.rs`); `arch/x86-generic/src/multiboot.rs` and its
  module line. Per C2: `BOOT_INFO_MAGIC` becomes the only accepted entry, a
  loud panic otherwise; fold `trim_to_safe_memory`'s kernel-end clip into the
  `kernel::platform::bootinfo` consumer that already performs one. Check the
  per-crate unsafe baselines — expect counts to **drop**; ratchet down in the
  same commit (`check-style`/unsafe baselines only ratchet down per policy).
- **R4 — docs and harness rewrite:** `docs/architecture/real-hardware-x86_32.md`
  boot recipe becomes the chain recipe (write `target/rosx-i486-boot-disk.img`
  to IDE/CF; BIOS needs CHS-or-LBA INT 13h — proven on 1996 Award; the era-media
  GRUB paragraphs move to the decision record's history); README platform line;
  `docs/architecture/boot-media-and-harness.md` and `486-baseline.md` stage-audit
  sentences; `arch/x86-86box/README.md` — harness survives as the **chain
  era-verification** path (V-step protocol), GRUB sections deleted or marked
  historical; `configs/machines.md` baseline row re-pointed at the chain verdicts;
  `docs/evidence-checklist.md` media line (no more `build-legacy-hdd.sh` media).
- **R5 — closeout:** delete `docs/todo/multiboot-retirement.md` in the commit
  that completes the work; final `cargo xtask verify` on both arches; `retro`
  step-end report per R-step.

Out of scope for R: x86_64 (its GRUB removal completed in swap M4); QEMU suites;
`system::boot_info` treaty (frozen, unchanged); the 86Box harness itself.

## 5. Risks

- **Firing R before V1/V2** loses the only era-boot story for the AMI/Phoenix
  families while their chain branches (E820 synthesis, DL quirk) remain
  unproven. Mitigation: R requires the trigger; V results should be attached to
  the decision record either way.
- **Real-silicon recipe after R** relies on the chain on hardware. The rehearsal
  *strengthened* this (CHS fallback proven on a 1996 BIOS; GRUB Legacy needed a
  maintained, ISA-audited third-party stage set). The silicon checklist stays
  open regardless — no claim beyond rehearsal facts is made anywhere.
- **86Box-mcp `record_start` is broken on this host** (NumPy C-extension
  ImportError in the server venv, 2026-11-21): PNG frames must be captured from
  the inline `screenshot` until the MCP env is fixed; V-step evidence protocol
  should expect `screen.txt` as primary.
- **Disk growth step** (image → cfg geometry) is a rehearsal artifact; V3 removes
  it from the critical path, and R never depends on grown images.

## Friction log

    rehearsal | `mdir -i hdd.img@@2048 ::` said "non DOS media"; @@ offset is bytes (LBA 2048 = @@1048576), not sectors | ENV-TRAP | 1 round
    rehearsal | stage2 banner burst scrolled off between two 1000 ms steps; needed a third boot pass at 100 ms | MISSING-FACT | 1 extra boot pass
    rehearsal | `record_start` ImportError: NumPy C-extension broken in the 86box-mcp server venv | ENV-TRAP | 0
    rehearsal | `fpu_registers` reports `no FPU` by raising, aborting a combined probe call | ENV-TRAP | 1 round
    rehearsal | `nm ... | grep ' SYSTEM_TIME_MS$'` (evidence-checklist.md) finds nothing — symbol is mangled in today's build | STALE-DOC | 1 round
    rehearsal | README era bar (2026-11-18, fake kernel) reads as if the production chain were era-verified | STALE-DOC | 0

### Rehearsal notes (prose)


- `gotchas.md` did its job: container launcher `binary=`, strays check, primed NVR
  — zero surprises on that axis.
- New traps worth a `gotchas.md` line later: mtools `@@` offset is **bytes**
  (`@@1048576` = LBA 2048, not `@@2048`); the stage2 ELF-load window is ~3–4 s
  of emulated time and the banner burst then scrolls in <150 ms — capture with
  100 ms stepping or miss it; `vm_create` lays a blank disk that must be
  overwritten *after* creation; `fpu_registers` reports `no FPU` by **raising**
  (expected, but it aborts a combined probe call — run it last); `nm` symbol for
  the PIT counter is mangled (`SYSTEM_TIME_MS` grep needs the mangled suffix or
  `-P` pattern), so `evidence-checklist.md`'s derivation line needs an update in
  the R4 sweep.
- `boot32.img`-era bar vs production bar conflated at first read (see §1) — the
  README should distinguish the two lines when it is re-pointed (R4).
