# Text editor

## Invariants

- `ted` is a disk app, not in the ROM or the boot plan.
- Saves are delete-then-recreate because the filesystem engines have no truncate; lines are saved LF-terminated.
- `^C` is not a `ted` key: the shell's foreground watcher kills it hard; `^Q` is the guarded exit.

## Design

Text editor — `ted` is a pico-style full-screen single-file editor (a disk app, not in ROM or the boot plan): a byte-transparent `Vec<Line>` buffer with LF-terminator save policy (a trailing `LF` loads without a phantom line, save always terminates lines with `LF`), a diffing ANSI renderer on the 80×25 grid that paints its own block cursor (the kernel drives no `0x3D4` and the ANSI subset has no SGR reverse), a VFS-client file layer with **delete+recreate shrink-safe save** (the engines have no truncate), and a quit prompt guarding unsaved changes. Untitled buffers answer `^S needs a file name` — no save-as overlay in v1. Keymap: `^F/^B/^N/^P` (+arrows) move, `^A/^E` line edges, `^D`/Backspace delete (line joins at edges), `^S` save, `^Q` quit; Left/Right cross line boundaries, Up/Down clamp without a sticky column. `^C` is not a ted key — the shell's foreground watcher `Kill`s ted hard (no save prompt); `^Q` is the guarded exit. Proven on both arches by `tests-integration/tests/ted_edit.rs` (create/grow/shrink saves + quit prompt verified byte-for-byte from the FAT image via `mtype`); plan `plans/ted-editor-plan.md`, record `plans/ted-editor-impl.md`.
