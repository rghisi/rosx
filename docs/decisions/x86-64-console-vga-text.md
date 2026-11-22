---
status: decided
date: 2026-11-22
revisit-when: the VESA framebuffer successor work starts (docs/todo/vesa-framebuffer-successor.md), or stage2 grows a VBE mode-set
---

## Question
The x86_64 kernel rendered its console onto a linear framebuffer handed over by
the `bootloader` crate. The chain's BootInfo supports `DisplayKind::VgaText` at
`0xB8000`, but stage2 performs no VESA mode set; what console does the chain
boot give the 64-bit port?

## Chosen
VGA text. The x86_64 entry multiplexes `x86_generic::vga_buffer` (identity base
0xB8000, always mapped by the chain's identity map) with the 0xE9 debug
console — the same output story the x86_32 port and the boot chain itself
already use. `framebuffer.rs`/`terminal_fonts.rs` stay in the tree, detached:
compiled, unreferenced, `#[expect(dead_code)]`, re-signatured off
`bootloader_api` types to a plain `(buffer_start, width, height, stride,
bytes_per_pixel, bgr)` init so a successor can wire them without archaeology.

## Alternatives
Grow stage2 with a VBE mode set filling `DisplayKind::LinearFramebuffer` now:
rejected — real-mode VBE work in the loader is a project of its own, and the
chain's frozen ABI stays testable without it. Delete the framebuffer code:
rejected by user decision — it is the seed of the successor.

## Consequences
`kernel::platform::bootinfo` validates `DisplayInfo` VgaText@0xB8000 on both
ports; no display block can appear on the wire that the kernel cannot honor.
The revived plan lives in `docs/todo/vesa-framebuffer-successor.md`.
