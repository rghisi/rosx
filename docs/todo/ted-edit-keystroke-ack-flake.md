---
priority: normal
date: 2026-11-22
trigger: the next `ted_edit` integration failure (any arch or CI run)
---

# ted_edit keystroke acknowledgement flakes intermittently

`tests-integration/tests/ted_edit.rs` `send_key_awaited` gave up after 4 attempts x 6 s waiting
for ted's `^S Save` status line while launching (`ted_shrink_save_rewrites_the_shorter_file`
failed once inside `cargo xtask verify` and once in a solo `ted_edit` binary run). Repetition
afterwards was green: 8/9 new-kernel binary runs, 6/6 pre-change-kernel binary runs, 5/5 isolated
test — so the flake is not attributable to the 2026-11 ELF-load-at-syscall change; the keystroke
delivery path (QEMU stdin -> ps2d -> foreground app) is the suspect. When it next fails, capture
the full session transcript (the helper currently panics without dumping it) before retrying, and
look there first.
