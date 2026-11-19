# ted — Full-Screen Text Editor — Implementation Record

> **Status:** Complete on branch `ted-editor` (base `35b98d4` = `origin/main`), companion to
> `plans/ted-editor-plan.md`. Written 2026-11-19. Every decision taken during implementation —
> including the ones made autonomously after the owner approved the plan — is recorded here.

## Milestones → commits

| step | content | commit |
|---|---|---|
| Plan | `plans/ted-editor-plan.md` | `5a0b020` |
| M1 | Input amendment: `input-protocol` sentinels + predicate, `ps2d` Ctrl/Esc/arrow decoding, shell filter | `3272e62` |
| M2/M3 | `apps/ted`: line buffer model, keymap, viewport math, diffing screen layer (33 host unit tests) | `5901c34` |
| M4 | VFS file layer, shrink-safe save, quit guard, `main.rs` loop, app registration (workspace + `USER_APPS` 41→42) | `eb3e437` |
| M5 | `sendkey` Ctrl/Esc/arrow tokens, host-side FAT image read, `QemuSession::shutdown`, `ted_edit` suite (3 tests × both arches) | `50e977a` |
| M6 | This record, plan amendment, AGENTS.md roster + input architecture + editor doctrine | this branch |

## Evidence

- **Host unit tests**: `input-protocol` 9, `ps2d` 19, `shell` 11, `ted` 33 — all green; full
  `cargo xtask test --skip-integration` green.
- **Build surfaces**: `cargo xtask build` and `cargo xtask build x86_32` complete at zero
  warnings (the toolchain's own `core` future-incompat report aside); `cargo xtask check-isa
  x86_32` reports **42 user ELFs 486-clean** (includes `ted`).
- **Integration x86_64** (`ROSX_KERNEL_ELF=target/rosx/release/rosx`): `ted_edit` 3/3 green —
  create + save + clean-exit verified byte-for-byte via host `mtype` (`Hello ted\n`), shrink save
  (delete+recreate on rxfat-mounted FAT16 root: `abcdefgh\n` → `abcde\n`), quit prompt `n`→return
  `y`→save+exit (`dirt\n`).
- **Integration x86_32**: `ted_edit` 3/3 green; `boot_and_shell` 4/4; `disk_boot` 3/3.
- **Regression (x86_64)**: `boot_and_shell` 4/4, `exit_status` 5/5, `shell_background` 5/5,
  `shell_command_arguments` 1/1 — the shell filter and decoder amendment are invisible to
  plain-text sessions, as designed.

## Decisions (options considered, choice, why)

1. **Command-key delivery = amend `ps2d`** (owner-approved). `Ctrl+[a-z]` → `0x01..0x1A`,
   Escape → `0x1B`, arrows → PUA `U+E010..U+E013`. Alternatives considered: vi-modes on the three
   surviving control chars (awkward), and a self-contained crippled editor in `apps/` (rejected).
   The wire is unchanged — chars are chars.
2. **Ctrl + punctuation / Ctrl + Escape stay silent** (option: terminal-style `Ctrl+[` → Escape —
   rejected: the Escape sentinel must have exactly one origin, and `0x1B` from two keys would
   alias). Right Ctrl (`E0 14`) is not tracked — Left Control only; extended non-arrow keys stay
   silent exactly as before the amendment.
3. **The treaty names only what is new**: `KEY_ESCAPE`, `KEY_ARROW_*`, `is_arrow_sentinel`,
   `is_command_char` in `input-protocol::wire`. Named per-control-code constants (26 of them) were
   rejected as treaty bloat — control codes are standard ASCII; `apps/ted/src/keys.rs` maps them
   with documented literals.
4. **The shell filter swallows `'\t'` too** (option: exempt Tab at the prompt — rejected: tabs in
   command lines have no use in this shell and the predicate stays one honest rule: `c < 0x20`,
   DEL, or a sentinel). `'\n'` and `'\x08'` are handled before the filter, so Enter/backspace are
   unaffected.
5. **Shrink save = delete + recreate, grow/equal = direct rewrite** (owner-approved). The crash
   window between delete and create is accepted for v1 (documented in the plan); the honest fix is
   an engine-truncate plan, not a rename-in-VFS workaround, and neither belongs to `ted`.
6. **An untitled buffer cannot save**: `^S` answers `^S needs a file name: run ted <path>` on the
   status line. This amends the plan's earlier "untitled bound to argv[1] on first save" sentence,
   which conflated the *no-argument* case with the *missing-file* case — `ted /new.txt` binds the
   path immediately (`NotFound` → empty buffer bound to it); `ted` alone has no name to bind.
   A save-as prompt line is deferred v1.5 (needs an in-editor input overlay).
7. **No wrap; horizontal scroll** (per plan). Left/Right cross line boundaries (pico behavior);
   Up/Down clamp the column with **no sticky preferred column** (deferred — clamping is honest and
   stateless).
8. **Newline policy**: load splits on `LF`, a trailing `LF` does not produce a phantom last line,
   an empty file is one empty line; save always writes `LF` line endings with a trailing `LF`
   (so an empty buffer saves as one byte). CRLF input keeps its `CR` bytes inside the line — the
   buffer is byte-transparent; normalization is LF *terminators* only.
9. **Display is byte-cell-oriented (ASCII-first)**: a UTF-8 multi-byte character occupies several
   cells; the frame string is built lossy at paint time (display approximation only — the buffer
   and saved bytes are untouched). Fixing display width is tied to a terminal-aware console future,
   not to `ted`.
10. **Cursor is painted, not hardware**: the kernel drives no `0x3D4` and the ANSI subset has no
    SGR reverse, so the cursor cell is re-emitted as `ESC[47m ESC[30m<cell> ESC[0m` — a white
    block showing the character under it. The renderer's dirty-row diff always touches the old and
    new cursor rows, so no stale block survives a move.
11. **Renderer is a dirty-row diff over a whole-row model**: `ESC[K` (whole-line clear, the parser
    ignores `K` modes anyway) + row content, one batched `Print` per frame; the block cell is
    re-emitted each frame. At 80×25 this is far below the console's throughput; the earlier
    concern that `ESC[K` lacks to-EOL semantics is moot because every dirty row repaints whole.
12. **Integration keys are acknowledged by observable effect, not echo**: raw `sendkey` bursts
    were the initial suspect for dropped backspaces, so every control key in `ted_edit` is gated
    on its visible consequence (status `Col n`, a `saved N bytes` notice, the returned `rose>`)
    with up to 4 attempts. Root-cause note recorded honestly: the "dropped" backspaces turned out
    to be all delivered — the first assertion was arithmetically wrong (3 backspaces on 8 chars
    yield 5 chars). The acknowledgement loop stays: it makes sessions deterministic regardless of
    i8042 pacing and fails loudly instead of silently.
13. **Host-side verification plumbing**: `QemuSession::shutdown()` (clean monitor quit + reap,
    tempdir retained) + `fat_disk::read_file_from_image` (`mtype` on the `image@@32256` offset).
    Reads happen after a clean QEMU exit, so host page-cache staleness cannot fake a result.
14. **Engine coverage**: the integration proofs exercise the FAT16 root volume (`rxfat`) —
    including the shrink delete+recreate there. A dedicated rxfs save session was not added:
    `FORMAT`/`MOUNT` of a scratch disk belongs to the existing `vfs_hdd_flow` topology and the
    shrink path rides identical `vfsd` opcodes; rxfs `remove`/`create` are covered by the engine
    suites. Recorded as a conscious coverage boundary.
15. **Manifest hygiene**: `input-protocol` moved to `ps2d`'s regular `[dependencies]` (the decoder
    lib needs it for host tests; the `none`-target section keeps only `usrlib`). `ted` registered
    in `Cargo.toml` members + `USER_APPS` (now 42); deliberately **not** in `BOOTSTRAP_APPS` or
    the boot plan.

## Known limits (by decision, not oversight)

- No cut/paste, help overlay, undo, search, word motions, wrap, multi-buffer (each a named
  future; the `Command` enum is the attach point).
- Delete+recreate crash window on shrink saves (engine-truncate plan is the real fix).
- No sticky cursor column; ASCII display cells; untitled save hint instead of a save-as prompt.
- Right Ctrl untracked; extended non-arrow keys silent.
