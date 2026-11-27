---
status: decided
date: 2026-11-22
revisit-when: nothing on the roadmap; the custom chain is now the only boot path on both x86 ports
---

## Question
The x86_64 port booted through the third-party `bootloader` crate (a BIOS
carrier image built by `arch/x86-runner`, `bootloader_api` types at the kernel
entry). Once `arch/x86-bootloader` proved its chain end to end — including the
64-bit handoff and the real-kernel boot from the unified FAT disk — should the
carrier survive as the production boot path?

## Chosen
Retire `bootloader` and `bootloader_api` entirely (`plans/x86_64-carrier-swap.md`,
commits `9052850`/`1afa661`). Both ports chain-boot the same unified FAT disk:
stage1 MBR, `ROS2` stage2 blob at LBA 8 (flavor per arch), bootable FAT16 slice
at LBA 2048, `KERNEL.ELF` at the volume root. `arch/x86-runner` keeps only the
disk-path derivation, the QEMU wiring and the harness attachments; the kernel
entry is the chain's `BOOT_INFO_MAGIC` handoff decoded by
`kernel::platform::bootinfo`. CI publishes `target/rosx-boot-disk.img` where the
carrier image used to be.

## Alternatives
Keep the carrier as a second medium behind `BootMedium::DiskImage`; rejected —
two media means two kernel entry contracts, a secondary-IDE-bus invariant with
no owner, and a dependency the 486-era story never needed. Chain-load the
carrier from the unified disk; rejected — one more stage for no capability.

## Consequences
The NXE workaround (`clear_nx_bits`), the `physical_memory_offset` plumbing and
the APIC execute-disable hack retire with the carrier (the chain's tables never
set NX). The kernel installs its own copy of the frozen GDT because stage2's
image region is published `BootloaderReclaimable`. The link shape is pinned by
`docs/decisions/x86-64-link-code-model-large.md`, the console by
`docs/decisions/x86-64-console-vga-text.md`. Era-hardware verification of the
chain stays open for both arches (AGENTS.md hardware evidence rules).
