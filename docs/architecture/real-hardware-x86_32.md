# Real hardware (x86_32)

## Invariants

- No checklist item may be marked green without hardware evidence.
- The x86_32 port stays on the 486 baseline: no FPU state, no `cpuid`, atomics of at most 32 bits, and the pinned feature strings as the floor.

## Recipe and checklist

Boot recipe: `cargo xtask build x86_32` then write `target/rosx-i486-boot-disk.img` whole onto an IDE disk or CF adapter (`dd if=target/rosx-i486-boot-disk.img of=/dev/sdX bs=4M conv=fsync`) — the disk carries stage1 in the MBR, stage2 and `KERNEL.ELF` on the FAT16 volume, so no external bootloader is needed on the board. The emulator-side rehearsal lives in `arch/x86-86box/` (its README is the runbook; the production chain booted to `rose>` there on AMI, Phoenix and Award firmware). Every QEMU x86_32 session already runs `-cpu 486`, and `cargo xtask check-isa x86_32` audits the shipped ELFs; silicon still needs its own evidence. **Hardware verification checklist — every item open until executed on a board; no item may be marked green without hardware evidence:**

1. Memory detection through the stage chain: `[BOOTINFO] regions=…` reports a plausible total for the board (4–64 MB); firmware that refuses E820 must still boot through the E801 and INT 88h synthesis branches (both proven on 86Box firmware, never yet on silicon).
2. `PAUSE`-as-NOP: no `#UD` at first `Once` contention during early boot.
3. PIT 100 Hz / 8259A cascade timing on VLB/PCI chipsets: `SYSTEM_TIME_MS` advances, shell echo works.
4. 16-bit IDE PIO on a real drive: atad identify + LBA read (timing margins, IORDY).
5. VGA text console standalone: writes to the unclaimed debug port 0xE9 must stay harmless; VGA is the only output.
6. The `rep bsf` idiom (objdump prints `tzcnt`): executes as `BSF` (F3 ignored) on silicon.
7. 486SX sub-case (only if SX support is ever chosen): boot with no FPU present.
