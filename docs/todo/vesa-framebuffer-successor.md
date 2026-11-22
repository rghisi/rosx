---
priority: normal
date: 2026-11-22
trigger: display work beyond VGA text is scheduled
---

# VESA framebuffer successor for the x86_64 port

The 2026-11-22 carrier swap chose VGA text for the chain boot
(`docs/decisions/x86-64-console-vga-text.md`) and kept the glyph renderer
alive in the tree but detached: `arch/x86_64/src/framebuffer.rs` and
`terminal_fonts.rs` compile, are unreferenced (`#[expect(dead_code)]`), and
`framebuffer::init` already speaks plain numbers —
`(buffer_start, width, height, stride, bytes_per_pixel, bgr)` — not
`bootloader_api` types.

Successor work: grow stage2-64 (stage2-32 later, if wanted) with a real-mode
VBE mode set (`INT 10h AH=00h/BH=0` after a VESA info probe, LFB from the mode
info block), publish it as `DisplayKind::LinearFramebuffer` in the frozen
BootInfo treaty, and wire `framebuffer::init` at the kernel entry with a
VGA-text fallback when the mode set failed. The BootInfo wire already carries
the display block and `kernel::platform::bootinfo` decodes both kinds; the
`boot-test` corpus needs a checkpoint that asserts a VBE banner line from
stage2-64 before the kernel boots. Era-hardware questions (VBE presence on
486/early-Pentium BIOSes) belong to the same effort.
