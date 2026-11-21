---
priority: normal
date: 2026-11-21
trigger: era-media retirement on real hardware, or any decision dropping GRUB Legacy boot from the x86_32 support matrix
---

# Multiboot deletion sweep (C2 of the carrier swap)

x86_32 now chain-boots the unified disk (`plans/x86_32-carrier-swap.md`), but the Multiboot
path stays because GRUB Legacy era media boot through it: `arch/x86-86box/scripts/build-legacy-iso.sh`
and `build-legacy-hdd.sh` load the kernel via its Multiboot header, so three pieces must stay:

- the `.multiboot` header section in `arch/x86_32/src/boot.S`;
- the `0x2BADB002` branch of `kernel_main` in `arch/x86_32/src/main.rs`;
- `x86_generic::multiboot::usable_memory` (`arch/x86-generic/src/multiboot.rs`).

Sweep when era media leave the support matrix (the 86Box `b32` / real-silicon verdicts per
AGENTS.md hardware rules): delete all three, make `BOOT_INFO_MAGIC` the only accepted entry
with a loud panic otherwise, and fold `trim_to_safe_memory`'s kernel-end clip into the
`kernel::platform::bootinfo` consumer that already performs one.
