# Real hardware (x86_32)

Boot recipe: copy `target/rosx-i486/release/rosx-x86_32` (a Multiboot v1 ELF) onto an IDE disk or CF adapter carrying GRUB Legacy 0.97 or GRUB 2, and chain to it — GRUB Legacy: `kernel /boot/rosx` then `boot`; GRUB 2: `multiboot /boot/rosx` then `boot`. This era path is now scripted: `cargo xtask era-image x86_32` produces the GRUB Legacy 0.97-67 FAT16 HDD image (the recipe below, executed literally) and the El Torito ISO, and the emulator-side rehearsal lives in `arch/x86-86box/` (its README is the runbook). Every QEMU x86_32 session already runs `-cpu 486`, and `cargo xtask check-isa x86_32` audits the shipped ELFs; silicon still needs its own evidence. **Hardware verification checklist — every item open until executed on a board; no item may be marked green without hardware evidence:**

1. Memory detection through a v1 bootloader: `[MEMORY]` banner appears after GRUB; on a GRUB build that lacks the mmap buffer, the `mem_lower`/`mem_upper` fallback must still yield a usable heap (`[MEMORY] Total:` on 4–64 MB expected).
2. `PAUSE`-as-NOP: no `#UD` at first `Once` contention during early boot.
3. PIT 100 Hz / 8259A cascade timing on VLB/PCI chipsets: `SYSTEM_TIME_MS` advances, shell echo works.
4. 16-bit IDE PIO on a real drive: atad identify + LBA read (timing margins, IORDY).
5. VGA text console standalone: writes to the unclaimed debug port 0xE9 must stay harmless; VGA is the only output.
6. The `rep bsf` idiom (objdump prints `tzcnt`): executes as `BSF` (F3 ignored) on silicon.
7. 486SX sub-case (only if SX support is ever chosen): boot with no FPU present.
