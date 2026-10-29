# Why User ELFs Are ~24KB, and the Measured Path to ~2KB

> **Investigated:** 2026-10-27, on `main` (`8afb024`), toolchain `nightly-2026-10-03`.
> **Question:** the smallest user app (`randomd`, a bind/connect/send loop) compiles to a ~20KB ELF. Where do the bytes come from, and how small can we get?
> **Bottom line:** app logic is 1–3KB of each binary. The rest is ELF container waste (~10KB/app) and `core`/`alloc` instantiations (~6–14KB/app) that the build settings and one `usrlib` design choice pull in. A settings-only change takes `randomd` from **19,712 → 1,816 bytes** and the ROM from 455KB → **154KB**, with the full integration suite passing.

## Reproducing

```bash
cargo xtask apps                      # builds the 15 ROM apps + shell, release profile
ls -la target/rosx-user/release/
size -A target/rosx-user/release/randomd
readelf -SW target/rosx-user/release/randomd
readelf -lW target/rosx-user/release/randomd   # program headers, padding
nm -S -C target/rosx-user/release/randomd | sort -rn -k2   # per-symbol sizes (unstripped builds only)
```

Baseline sizes (bytes, `cargo xtask apps`, no other flags):

| app | size | app | size | app | size |
|---|---:|---|---:|---|---:|
| randomd | 19,712 | | snake | 26,072 | | ping | 32,296 |
| mem | 20,776 | | threads | 26,240 | | pi | 36,200 |
| conway | 22,304 | | tetris | 26,792 | | vfs | 41,680 |
| inputd | 22,368 | | pong | 29,952 | | vfsd | 83,256 |
| echo | 22,384 | | bulk | 22,696 | | ps2d | 22,688 |

The 15 ROM-app bytes sum to **455,416** (shell adds 58,312 but is embedded by the arch crate, not packed). `cargo xtask rom` packs the **entire file** of each ELF into `rom.img` — including parts the loader never reads.

## Anatomy: four sources of container waste

`randomd` file breakdown (19,712 bytes):

| Bytes | Component | Needed by the kernel loader? |
|---:|---|---|
| 3,640 | padding: `usrlib/linker.ld` starts text at vaddr 0 (`. = 0;`), so lld pushes ELF headers to their own page — first `PT_LOAD` sits at file offset `0x1000` | — |
| 7,055 | `.text` | yes (`PT_LOAD`) |
| 1,052 | `.rodata` + small dynamic sections | yes |
| 1,944 | `.symtab` | **no** |
| 3,768 | `.strtab` | **no** |
| 115 | `.shstrtab` | **no** |
| ~2.2K | ELF/program headers + section headers | headers yes, section headers no |

`kernel/src/elf/mod.rs::load_elf64` reads only the ELF header, program headers, and `PT_LOAD`/`PT_DYNAMIC` segments, copying each segment from `p_offset` to `p_vaddr` in the allocated image and computing `entry = base + e_entry`. Everything else in the file — symbol tables, section headers — is dead weight in the ROM (it costs ROM bytes, not RAM).

So even before looking at code, ~6KB of every app is symbol tables and ~4KB is page padding.

## Anatomy: what is inside `.text`

The app's `main` is typically 0.3–1.3KB. The rest is `core`/`alloc` machinery instantiated per app, because there is no shared user runtime:

- **`core::fmt` (~3–7KB/app).** `usrlib`'s `print!`/`println!` route to `Syscall::print(fmt::Arguments)`, which calls `alloc::fmt::format(args)` — a heap `String` built through the full `Formatter` (`pad`, `pad_integral`, `PadAdapter`, every `Display` impl used) plus `String`/`RawVec` growth logic. In baseline `pi` the `core::fmt` subsystem alone measures **6,534 bytes**.
- **Panic reporting (~2–4KB/app + strings).** `rust_begin_unwind` (690B), `slice_error_fail_rt` (907B), `str::count::do_count_chars` (930B), and panic-location strings embedding full toolchain paths (`/home/.../rust/library/core/src/str/mod.rs`).
- **Unicode tables (~4KB, only where `{:?}` formats a string).** `escape_debug` on `&str` pulls in `core::unicode::unicode_data` (`cn_planes_0_3::OFFSETS` 1,461B, `grapheme_extend::OFFSETS` 779B, maps + lookup functions; 4,031B total in `pi`). One line explains most of why `pi` is 36KB: `println!("args({}): {:?}", usrlib::argc(), usrlib::args());`
- **No LTO.** The release profile runs at `opt-level = 2`, `lto` unset, `codegen-units` unset, so each app links a slightly different inefficient copy of core/alloc — paid 15× across the ROM.

## Measured reductions

Each row cumulatively adds one lever (bytes per app; measured with scratch target dirs, repo settings otherwise untouched):

| cumulative config | randomd | snake | pi | vfsd |
|---|---:|---:|---:|---:|
| baseline | 19,712 | 26,072 | 36,200 | 83,256 |
| `strip = true` | 13,864 | 19,256 | 25,752 | 64,360 |
| + `lto = "fat"`, `codegen-units = 1` | 8,184 | 19,272 | 23,824 | 60,656 |
| + `opt-level = "z"` | 11,352 | 17,584 | 21,984 | 54,016 |
| + `-C panic=immediate-abort` | 5,400 | 12,648 | 16,848 | 36,568 |
| + `linker.ld`: `. = SIZEOF_HEADERS;` | **1,816** | **9,128** | **13,328** | **32,984** |

Notes on individual levers:

- **`strip = true`** removes `.symtab`/`.strtab` (~6KB/app) at zero behavioral cost.
- **LTO + single codegen unit** dedups and dead-strips cross-crate generics; the main win on small apps (−5.7KB on `randomd`).
- **`opt-level = "z"`** helps large code, but alone it *regressed* `randomd` (8,184 → 11,352); net-positive in combination.
- **`panic=immediate-abort`** replaces panic formatting with trap instructions: −2.6 to −6.3KB/app, and drops the toolchain-path strings. **Trade-off:** `usrlib`'s panic handler no longer prints `panic at file:line` — panics become silent traps.
- **`. = SIZEOF_HEADERS;`** reclaims the 4KB header page. `-z noseparate-code` alone changes nothing while `. = 0;` pins text to vaddr 0; the linker script must start at `SIZEOF_HEADERS` so lld can fold headers into the first page. The final layout (`PT_LOAD` at file offset 0, `entry = 0x200`) loads correctly under the current kernel loader because it is offset-based, not page-based.

## Verification of the final configuration

Built all 16 ELFs with the cumulative config — `rom.img` = **154,624 bytes** — then ran the integration suite against the shrunken binaries:

| app | baseline | final | | app | baseline | final |
|---|---:|---:|---|---|---:|---:|
| randomd | 19,712 | 1,816 | | tetris | 26,792 | 10,176 |
| mem | 20,776 | 5,488 | | pong | 29,952 | 10,112 |
| conway | 22,304 | 7,328 | | ping | 32,296 | 13,704 |
| inputd | 22,368 | 3,336 | | pi | 36,200 | 13,328 |
| echo | 22,384 | 5,496 | | vfs | 41,680 | 20,856 |
| ps2d | 22,688 | 3,288 | | vfsd | 83,256 | 32,984 |
| snake | 26,072 | 9,128 | | **ROM** | **~455K** | **154,624** |
| bulk | 22,696 | 6,736 | | threads | 26,240 | 10,632 |

Integration results (headless QEMU): `tests-integration` unit tests **18 passed**; `app_lifecycle` (snake, tetris idle) **2 passed**; `boot_and_shell` **4 passed**; `elf_arguments` (kernel→app argc/argv — validates the new entry/vaddr layout) **1 passed**; `ipc_bulk`, `ipc_ping`, `ipc_random`, `ipc_vfs`, `kstats_mem`, `shell_command_arguments` roundtrips **6 passed**. No failures. The apps exercising the heap, IPC data paths, kstats, and argument passing run identically at the smaller sizes.

What remains in a final-config `pi` (unstripped measurement): `core::fmt` subsystem ≈ 4.2KB, unicode tables ≈ 3.7KB (still from the `{:?}` on args), app `main` ≈ 1.3KB.

## Proposals

In order of value/risk. Steps 1–3 are settings-only; steps 4–5 touch `usrlib`/apps.

1. **Size profile.** Add to `Cargo.toml`:
   ```toml
   [profile.release-small]
   inherits = "release"
   opt-level = "z"
   lto = "fat"
   codegen-units = 1
   strip = true
   ```
   and have `xtask` build apps with `--profile release-small` (`pack_rom` and `arch/x86_64/build.rs` paths follow). Keep the kernel on `release`.
2. **Reclaim the header page.** `usrlib/linker.ld`: `. = SIZEOF_HEADERS;` plus `-z noseparate-code` in `.cargo/config.toml`'s `[target.rosx-user] rustflags` (add, don't replace — see pitfall below).
3. **`panic=immediate-abort` for apps only.** The current cargo cannot parse this value from a manifest profile; pass `-C panic=immediate-abort` via the `[target.rosx-user] rustflags` list. Recommended only for the size profile, so dev builds keep `panic at file:line` messages.
4. **`usrlib::print` without `String`.** Replace `alloc::fmt::format(args)` with a fixed stack-buffer `fmt::Write` (flush → `Print` syscall when full). Removes `String`/`RawVec`/allocator paths from every print site; app code is unchanged.
5. **Avoid `{:?}` on `&str`.** The ~4KB unicode tables exist only to escape-debug strings. A tiny `usrlib` helper for printing args (or plain `Display` in apps) drops them wherever used — ~4KB on `pi` alone.

Combined estimate: floor under 2KB, typical apps 5–10KB, ROM ≈ 154KB (−66%), with steps 4–5 pushing further.

## Pitfalls found along the way

- **`RUSTFLAGS` env overrides, not extends.** Setting `RUSTFLAGS` silently replaces `[target.rosx-user] rustflags` from `.cargo/config.toml`, losing `-Tlinker.ld --pie` and producing broken binaries that still "link". Experimenting with size flags must replicate those entries (or edit the config file).
- **`-z noseparate-code` with `. = 0;` is a no-op** — the custom linker script forces the separate header page; only the script change reclaims it.
- **Stripped binaries hide the analysis:** `nm`/`size` breakdowns need an unstripped build; keep a non-`strip` profile variant around for debugging.
- **`panic=immediate-abort` interacts with `usrlib::runtime::panic`** — the handler becomes unreachable dead code; if the config adopts it, consider removing the handler or documenting why it stays.
