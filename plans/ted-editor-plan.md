# ted — Full-Screen Text Editor — Implementation Plan

> **Status:** Implemented on branch `ted-editor` (M1–M6 complete; evidence and the full decision
> log in `plans/ted-editor-impl.md`). Originally written 2026-11-19 on branch `ted-editor`
> (base `35b98d4` = `origin/main`).
> **Feature:** `ted` — a pico/emacs-lite full-screen single-file text editor: open a path (or an
> untitled scratch buffer), move, insert, delete, save, quit — with a status line — plus the input
> amendment it stands on: `ps2d` learns Ctrl-as-control-codes, Escape, and arrow keys, the
> `input-protocol` treaty names the sentinels, and the shell stops echoing control junk.
> **Method:** every repo fact below was re-verified against this working tree (paths cited).

**Decisions (owner-approved in session 2026-11-19):**
- Command keys ride an **amended input stack**: `Ctrl+[a-z]` → `0x01..0x1A`, Escape → `0x1B`,
  arrows → Private-Use-Area sentinels, all delivered as `TAG_KEY` chars over the existing wire.
  Device semantics stays in `ps2d`, sentinel names live in `input-protocol` — architecture rules obeyed.
- **Shrink-safe save**: save serializes the whole buffer; when the new content is shorter than the
  on-disk size it **deletes the file and recreates** it (the engines have no truncate — fact 5).
- v1 is **core only**: move, insert/delete, save, quit, status line. The deferred capabilities are
  named non-goals below, each with a home.

---

## 1. Goal & Non-Goals

### Goal
- v1 delivers on **both x86_64 and x86_32**: `ted /path` edits and saves a file on the mounted FAT
  root volume (a `NotFound` path opens an empty buffer bound to it); `ted` with no argument edits
  an untitled buffer whose `^S` honestly reports that saving needs a file name.
- A scripted integration proof: keystrokes in via the QEMU monitor, saved file verified from the
  host through the FAT image.
- The input amendment is a clean, unit-tested, reusable surface: every future app gets Ctrl/Esc/arrows.

### Non-Goals (each a named future plan or a recorded decision)
- **Cut/paste (`^K`/`^U`), help overlay, search, undo, word motions**: deferred v1.5+; the `Command`
  enum is the seam they attach to.
- **Line wrap, multiple buffers, rename, block operations**: not in this design.
- **ROM / boot plan**: `ted` is NOT in `BOOTSTRAP_APPS` and gets no plan step; it ships on the FAT
  disks like every non-bootstrap app (fact 9). The 8.3 stem `ted` fits the boot-disk naming rule.
- **Kernel changes**: none. No ANSI extension (SGR reverse, save/restore cursor, IL/DL), no hardware
  cursor (`0x3D4` stays untouched), no new syscall, no VFS truncate opcode.
- **Engine truncate**: the delete+recreate save is the honest v1 answer; a real truncate across
  rxfs/rxfat + a VFS opcode stays its own plan.
- **CRLF**: buffers save LF-normalized (fact §4); no byte-preserving newline policy in v1.

---

## 2. Verified Baseline Facts (re-checked 2026-11-19)

1. **The INPUTD client surface is one `char`**: request tags `READ/READ_NOWAIT`, reply tags
   `KEY/NO_KEY`, key frame carries a `u32` codepoint (`protocol/input/src/wire.rs`), surfaced as
   `Syscall::read_char()` / `Syscall::try_read_char()` with a cached connection and reconnect
   retries (`usrlib/src/input.rs:14-64`). A control code or PUA codepoint is just another `char` —
   the amendment below needs **no wire change**.
2. **ps2d drops every command key**: `char_for` maps `Escape → None` and `LeftControl → None`
   (`apps/ps2d/src/decoder.rs:219,247`), and any extended scancode is swallowed because
   `raw if raw & 0x80 != 0 => None` also eats the `0xE0` prefix itself
   (`apps/ps2d/src/decoder.rs:20`). Only printables plus `'\n'`, `'\x08'`, `'\t'` reach apps.
3. **The shell echoes unknown chars into the command line**: the read loop handles `'\n'` and
   `'\x08'` and pushes *everything else* into the line buffer and the screen
   (`apps/shell/src/shell.rs:73-81`). Once ps2d delivers control codes, an unfiltered shell would
   print block glyphs and corrupt prompts — M1 filters them.
4. **The screen is an 80×25 cell grid driven by an ANSI subset**: `BUFFER_HEIGHT/WIDTH = 25/80`
   (`arch/x86-generic/src/vga_buffer.rs:56-57`); the parser implements `PrintChar`, SGR
   `{0, 30-37, 40-47, 90-97, 100-107}`, CUP (`H`/`f`), `J` (whole screen, mode ignored),
   `K` (whole line, mode ignored) — nothing else
   (`arch/x86-generic/src/ansi_parser.rs:93-152`). No SGR reverse, no save/restore cursor, no line
   insert/delete, and no `0x3D4` writes anywhere in the file — the hardware cursor does not exist,
   so the editor paints its own block cursor by swapping fg/bg on the cursor cell.
5. **rxfs writes only grow**: the size stamp is `if end > inode.size { inode.size = end }`
   (`fs/rxfs/src/volume.rs:440-441`); `grep -ri truncate fs/` finds no truncate API in any engine.
   The VFS wire does carry `DELETE` and open `MODE_CREATE`
   (`protocol/vfs/src/client.rs:93-115`), so delete+recreate is the shrink path available today.
6. **The typed VFS client is complete for ted's needs**: `connect/stat/read_whole/open/read/write/
   seek/close/delete` (`protocol/vfs/src/client.rs:83-266`); paths ride the 60-byte frame
   (`valid_path`).
7. **argv reaches apps**: the shell launcher passes arguments and `usrlib::runtime::args()`
   decodes them (`usrlib/src/runtime.rs:31`).
8. **The harness already scripts keystrokes**: `tests-integration/src/sendkey.rs` maps chars to
   QEMU monitor `sendkey` tokens (`ret`, `spc`, `backspace`, …) — it has **no Ctrl tokens yet**;
   every session attaches the mtools-built full-apps FAT master (`tests-integration/src/fat_disk.rs`)
   and the image is host-readable after the run, so a test can assert saved file contents.
9. **App registration is explicit**: workspace members are listed one by one
   (`Cargo.toml:20-63`), and `xtask/src/arch.rs:5` keeps `USER_APPS: [&str; 41]` — `ted` must be
   added to both (array length bumps to 42). `BOOTSTRAP_APPS` (`arch.rs:7`) stays as it is.
10. **The 486 floor does not bite**: the editor uses no floating point and no 64-bit atomics;
    `cargo xtask check-isa x86_32` gates the shipped ELFs regardless.

---

## 3. The Input Amendment (M1)

**`protocol/input/src/wire.rs`** — treaty constants, the one place apps read them from:
- `KEY_ESCAPE: char = '\u{1B}'`
- `KEY_ARROW_UP/DOWN/RIGHT/LEFT: char = '\u{E010}'..'\u{E013}'` (Private Use Area — never typed,
  never collides with real text)
- `is_command_char(c: bool)` — true for `c < ' '` and the sentinels; the shell's filter predicate.

**`apps/ps2d/src/decoder.rs`** — device semantics, unit-tested:
- track `ctrl: bool` from LeftControl make `0x1D` / break `0x9D` (Right Ctrl is extended — not v1);
- a two-state `awaiting_extended` flag for the `0xE0` prefix: extended makes `0x4B/0x48/0x4D/0x50`
  → arrow sentinels, extended releases (`code & 0x80`) and other extended makes are swallowed;
- `Escape → KEY_ESCAPE`;
- `Ctrl + [a-z]` → `0x01..0x1A` (Shift ignored while Ctrl is held); `Ctrl` + punctuation → `None`.

**`apps/shell/src/shell.rs`** — the else-branch ignores `is_command_char(c)`: nothing new reaches
the command buffer or the screen. No other INPUTD consumer reads characters (daemons don't), so the
blast radius is exactly these three files.

---

## 4. The Editor

### Keymap (v1)
| keys | action |
|---|---|
| `Ctrl+F` / `Ctrl+B` / `→` / `←` | char right / left |
| `Ctrl+N` / `Ctrl+P` / `↓` / `↑` | line down / up |
| `Ctrl+A` / `Ctrl+E` | line start / end |
| `Ctrl+D` | delete under cursor (joins the next line at EOL) |
| `Backspace` | delete before cursor (joins the previous line at col 0) |
| `Enter` | split the line at the cursor |
| `Tab` | insert spaces to the next 4-column stop |
| `Ctrl+S` | save |
| `Ctrl+Q` | quit; when modified, status-line prompt `Save modified buffer? (Y/N)` — `Y` saves and quits, `N`/`Esc` returns to editing; the prompt consumes its keystrokes, the buffer never sees them |

### Crate shape — `apps/ted/` (`#![no_std]` + `alloc`, pattern-copied from `apps/snake`)
- **`model.rs`** — the buffer: `Vec<Line>` with `Line = Vec<u8>` (byte-transparent — loaded bytes
  stay untouched, tabs included), cursor `(row, col-in-bytes)`, clamped moves, insert/delete/split/
  join, `display_width` (tabs expand to 4-column stops for screen math only). Pure → host unit tests.
  - Load: split on `'\n'`; a trailing `'\n'` does not create a phantom last line; empty file → one
    empty line.
  - Save: every line emits a trailing `'\n'` (documented LF normalization, §1 non-goal).
- **`keys.rs`** — `char → Command` (printable insert, the control codes above, the PUA arrows).
  Pure → host unit tests.
- **`screen.rs`** — rows 0..23 the text window with `(top_row, left_col)` scroll (no wrap), row 24
  the status bar: `ted /path  [+modified]  Ln x, Col y   ^S Save  ^Q Quit`. One batched frame per
  `print!`: per **dirty row** CUP + `ESC[K` + content; the old and new cursor rows are always dirty;
  the cursor cell is painted with swapped fg/bg (black-on-white block). Entry and exit run
  `ESC[2J ESC[H ESC[0m` so the shell gets a clean console.
- **`file.rs`** — `read_whole` on load: `NotFound` → empty buffer bound to the path (new file);
  any other error → message + exit code 1. Save: serialize, compare against the size observed at
  load/save time; grow-equal → `open(MODE_WRITE)` + write from 0; shrink → `delete` then
  `open(MODE_WRITE | MODE_CREATE)` + write (fact 5). Save errors print on the status line — the
  buffer and session survive a failed save.
- **`main.rs`** — `read_char` → `keys::command_for` → dispatch → render; the quit-prompt is a
  modal flag inside this loop.

### Registration (M2)
`Cargo.toml` members += `"apps/ted"`; `USER_APPS` += `"ted"` (`[&str; 41]` → `[&str; 42]`). No ROM,
no plan step (fact 9).

---

## 5. Milestones

- **M1 — Input amendment**: the three files of §3 with decoder unit tests (Ctrl state, full
  make+break streams for arrows, Shift+Ctrl, Ctrl+punctuation refusal, Escape). Acceptance: unit
  tests green; the keyboard-touching integration suites stay green on both arches (they type only
  plain text, so they must not notice).
- **M2 — Crate + model**: `apps/ted` skeleton wired per §4 registration, `model.rs` + `keys.rs`
  with host unit tests (`cargo test -p ted`): insert into empty buffer, split/join at every edge,
  clamped moves, tab display widths, save/load serialization round-trips.
- **M3 — Screen layer**: viewport, block cursor, status line, dirty-row render on a scratch
  buffer. Rehearsal on `cargo xtask run`: type, scroll vertically and horizontally, exit to a
  clean prompt — both arches.
- **M4 — File I/O**: load, new-file binding, grow save, shrink delete+recreate, dirty flag, quit
  prompt. Manual proof on both arches and both engines (the root volume is FAT; an rxfs volume on
  `ata0d1` covers rxfs): create, grow, shrink, re-load and see the bytes.
- **M5 — Integration test**: `sendkey.rs` gains `ctrl-<letter>` tokens; new suite `ted_edit`:
  boot → `ted /hello.txt` → type → `Ctrl+S` → `Ctrl+Q` → host reads the FAT image (mtools read
  helper beside `fat_disk.rs`) and asserts the bytes; plus the shrink sequence (type, backspaces,
  save) and the quit-prompt round trip. Runs on both arches, `--test-threads=1`.
- **M6 — Docs**: AGENTS.md app roster += `ted` + one kernel-subsystem line for the input
  amendment; this plan's status finalized.

---

## 6. Risks & Mitigations

- **Delete+recreate crash window** (file gone, new copy unwritten): documented in §1; acceptable
  for a v1 hobby editor; the real fix is the engine-truncate plan.
- **rxfat delete+recreate is an untested path**: `rxfat` create/write are proven, the *sequence*
  on a mounted FAT volume is not — M4 exercises it explicitly before M5 depends on it.
- **Shell regression**: control chars could leak into prompts if the filter misses a class —
  `is_command_char` is one predicate, unit-tested, used at the single read site (fact 3).
- **Extended-scancode stream shapes** (arrow make then `E0`+release): the decoder state machine is
  unit-tested over full press+release transcripts, not single codes.
- **`ESC[K` clears the whole line, not to EOL** (parser ignores modes): the renderer redraws whole
  rows anyway — the semantics match the render model exactly.
- **No hardware cursor**: the painted block cursor never blinks and vanishes on repaint — fine at
  80×25; a kernel cursor is its own future plan.
