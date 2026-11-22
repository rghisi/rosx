# Era machine selection

Selection criteria (plan §2.4), applied before spending any bring-up time:

1. accepts `cpu_family = i486sx` (the no-FPU floor — the harshest CPU the
   port must run on),
2. on-board IDE (the boot media are HDD/IDE),
3. RAM covering the era boards with headroom for the live VFS: the
   fixed 8 MiB rxfs volume proved hostile to an 8 MB board (5 MB heap),
   so the baseline ships 32 MB,
4. BIOS of the SX era,
5. a primed CMOS NVR shipped by 86box-mcp (tiebreaker: first boot needs no
   interactive SETUP session).

## Baseline

| id | machine | BIOS | CPU | FPU | RAM | IDE | CD boot | primed NVR | status |
|---|---|---|---|---|---|---|---|---|---|
| `486sp3c` | [SiS 496] ASUS PVI-486SP3 | Award PCI, 06/25/96 | i486SX-25 | none | 32 MB (template) | yes | **no** — Boot Sequence C,A only, no El Torito | yes | **verified baseline**: production unified disk boots RosX to `rose>` (grown 512 MB and the un-grown 64 MiB artifact as-is); E820 accepted; `fpu_registers` reports `no FPU` while the kernel runs FP-free |
| `vect486vl` | HP Vectra 486VL | AMIBIOS 4.01 AA.05.00 (VLSI 82C480) | i486SX | none | template | yes | no | template `ami-486-vectra.cfg` | production unified disk boots to `rose>`; refuses E820 *and* E801 — INT 88h synthesis proven; VLSI phantom-slave defect (gotchas.md) |
| `ecs486` | ECS UM8810P-AIO | Phoenix 4.04 PnP (UMC 8881 PCI) | i486SX | none | template | yes | no | template `phoenix-486-ecs.cfg` | production unified disk boots to `rose>` via EDD (`media=lba`); `DL=0x00` hand-off survives |

BIOS-era findings on `486sp3c` (settled by evidence, plan §2.4):

- The 1996 Award BIOS offers `Boot Sequence` C,A only — no El Torito. The
  CD cannot boot on this board.
- Era conclusion: on a board of this vintage **the HDD is the authentic
  RosX boot medium** — literally true now: the production chain boots the
  unified MBR disk (`target/rosx-i486-boot-disk.img`) written whole. The
  BIOS falls through blank-C to A:, so the boot floppy stays the
  era-authentic rescue path.

## Era CD boot (retired wish)

Late-486 PCI boards with `Boot Device: ATAPI CD-ROM` (Zida Tomato 4DPS,
Rise R418, FIC 486-VIP-IO2) were scoped to boot a `stage2_eltorito` ISO.
That media retired with the GRUB Legacy tooling
(`docs/decisions/multiboot-era-media-retirement-trigger-fired.md`), and the
production chain's stage1 is MBR-only, so era CD boot is not a boot
requirement — phase V4 was explored and skipped. The candidates stay as
historical notes only.

A future template differs from `486sx-pvi486sp3.cfg` only in `machine =`
(e.g. `machine = 4dps` for the Zida board); keep one template file per
machine, added only once its bring-up is verified.
