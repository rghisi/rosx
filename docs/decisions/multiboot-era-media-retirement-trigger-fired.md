---
status: decided
date: 2026-11-21
revisit-when: real-silicon results contradict the 86Box verdicts, or an era experiment needs a Multiboot-loading kernel (checkout tag last-multiboot-kernel)
---

## Question
`docs/todo/multiboot-retirement.md` (C2 of the carrier swap) fires its
trigger when era media leave the x86_32 support matrix — a maintainer
decision. Do they?

## Chosen
Yes, the trigger fires (maintainer decision, 2026-11-21 session), with
phase V (era verification of the production chain) executed first so the
decision is honest. Verdicts, all emulator evidence (silicon checklist
stays open per AGENTS.md):

- Award 4.51 `486sp3c`, grown unified disk: `media=chs` to `rose>` —
  `target/86box/runs/20261121T152239Z-486sp3c-chain/` (rehearsal).
- Award 4.51 `486sp3c`, **un-grown 64 MiB artifact** (as built): boots
  as-is, `rose>` x2 — `target/86box/runs/20261121T234310Z-486sp3c-ungrown/`.
- AMI 4.01 `vect486vl`: refuses E820 *and* E801 — INT 88h synthesis map
  proven end-to-end on era firmware; first boot exposed the VLSI
  phantom-slave defect, fixed by the DRDY presence rule in atad
  (commit 2c2cf95) — `target/86box/runs/20261121T231200Z-vect486vl/`.
- Phoenix 4.04 `ecs486`: `DL=0x00` hand-off survived, `media=lba` via
  EDD, e801 synthesis proven — `target/86box/runs/20261121T233107Z-ecs486/`.

## Alternatives
Stay V-only (GRUB Legacy media kept alongside the chain: two boot
stories to maintain, the 86Box harness still pointed at era media);
fire R on the rehearsal evidence alone (AMI/Phoenix chain branches
unproven — rejected as the plan's own risk section warns).

## Consequences
The C2 sweep executes in the same series: era-media tooling deleted
(GRUB Legacy scripts, `era-image`, the `--stages` ISA audit, CI stage
steps); the kernel accepts only `BOOT_INFO_MAGIC` at entry with a loud
panic otherwise (Multiboot header, the `0x2BADB002` branch and
`x86_generic::multiboot` deleted); docs and harness re-pointed at the
chain. Per-crate `unsafe` baselines drop (ratcheted in the same commit).
The annotated tag `last-multiboot-kernel` preserves the last tree that
boots through Multiboot for future era experiments. The real-hardware
recipe becomes the chain recipe (write the unified disk to IDE/CF —
proven on 1996-era BIOS emulations, CHS and LBA both).
