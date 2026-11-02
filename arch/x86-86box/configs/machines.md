# Era machine selection

Selection criteria (plan §2.4), applied before spending any bring-up time:

1. accepts `cpu_family = i486sx` (the no-FPU floor — the harshest CPU the
   port must run on),
2. on-board IDE (the boot media are HDD/IDE),
3. RAM covering the 8 MB era board and beyond,
4. BIOS of the SX era,
5. a primed CMOS NVR shipped by 86box-mcp (tiebreaker: first boot needs no
   interactive SETUP session).

## Baseline

| id | machine | BIOS | CPU | FPU | RAM | IDE | CD boot | primed NVR | status |
|---|---|---|---|---|---|---|---|---|---|
| `486sp3c` | [SiS 496] ASUS PVI-486SP3 | Award PCI, 06/25/96 | i486SX-25 | none | 8 MB (7168K extended) | yes | **no** — Boot Sequence C,A only, no El Torito | yes | **verified baseline**: GRUB Legacy 0.97-67 HDD boots RosX to `rose>`; GRUB floppy chain boots; `fpu_registers` reports `no FPU` while the kernel runs FP-free |

BIOS-era findings on `486sp3c` (settled by evidence, plan §2.4):

- The 1996 Award BIOS offers `Boot Sequence` C,A only — no El Torito. The
  CD cannot boot on this board.
- GRUB Legacy 0.97-67 cannot reach the ATAPI CD as a disk device either:
  a live `grub>` shell on the emulator returned `Error 23` for `root (cd)`
  (0.97 reads ISOs only through `stage2_eltorito`'s own boot-info table,
  which needs the El Torito boot the BIOS lacks).
- Era conclusion: on a board of this vintage **the HDD is the authentic
  RosX boot medium** — the AGENTS.md hardware recipe, scripted by
  `scripts/build-legacy-hdd.sh`. The BIOS falls through blank-C to A:, so
  the boot floppy is the era-authentic rescue path.

## Phase D candidates (era CD boot on a later-486 board)

The CD wish survives on late-486 PCI boards whose Award/AMI BIOSes gained
`Boot Device: ATAPI CD-ROM` — all SiS 496 / VIA-chipset 1997-98 designs,
all vetted as i486SX-accepting with IDE:

| id (86Box) | machine | expectation | primed NVR | verification |
|---|---|---|---|---|
| `4dps` | Zida Tomato 4DPS (SiS 496/497) | El Torito capable BIOS | no — needs one SETUP automation (F1 at checksum-error halt, set boot order, F10) | open |
| (TBD) | Rise R418 | El Torito capable BIOS | no | open |
| (TBD) | FIC 486-VIP-IO2 | El Torito capable BIOS | no | open |

Once the SETUP automation runs, 86Box saves the NVR and the board is as
reproducible as `486sp3c`. Success criterion: `rose>` reached from the
`stage2_eltorito` ISO (sizes-only MBI **without** a FLAGS patch), captured
with the §2.6 breakpoint flow; then this table gains the board's real BIOS
capabilities.

A Phase D template differs from `486sx-pvi486sp3.cfg` only in `machine =`
(e.g. `machine = 4dps` for the Zida board); keep one template file per
machine, added only once its bring-up is verified.
