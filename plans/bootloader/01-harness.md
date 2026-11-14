# Phase 1 — QEMU Feedback Harness (`cargo xtask boot-image` / `boot-test`)

Before any stage code exists, build the loop every later phase depends on: pack a boot image,
boot it headless, capture `COM1`, judge by exit code + stdout assertions. Lives in `xtask` (new
`boot_image.rs` / `boot_test.rs`), reusing the image primitives already in `xtask/src/boot_disk.rs`
(mtools invocation pattern, `rxpart::mbr` stamping) as library code — no parallel script farm.
Deliberately **not** built on `tests-integration`/`x86-runner`: those own the existing boot stack
and this plan must not touch them; the boot-test sessions speak COM1 + `isa-debug-exit` while the
existing suites speak port `0xE9` and pass `-serial none` (`tests-integration/src/qemu.rs`).

## 1. Exit-code law (normative, measured on QEMU 10.2.1)

| Guest action | QEMU exit code | Harness verdict |
|--------------|----------------|-----------------|
| write `0x10` to port `0xF4` | **33** | PASS (if stdout assertions also hold) |
| write `0x01` to port `0xF4` | **3** | FAIL (diagnostic path) |
| triple fault / reset with `-no-reboot` | **1** | FAIL — "crashed before verdict" |
| timeout (default 10 s), any other code | n/a | FAIL |

Measured fact behind the design: `0x00` written to `0xF4` yields exit code `1`, identical to the
`-no-reboot` triple-fault exit — so `0x00` is banned as a signal and `1` is defined as crash.
Each run records `qemu-system-* --version` into the log.

## 2. Standard invocations

32-bit (`i486`) media:

```bash
qemu-system-i386 -cpu 486 -m 32M \
  -drive format=raw,file=target/boot32.img \
  -serial stdio -display none \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -no-reboot
```

64-bit media:

```bash
qemu-system-x86_64 -cpu qemu64 -m 128M \
  -drive format=raw,file=target/boot64.img \
  -serial stdio -display none \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -no-reboot
```

No `-no-shutdown`: a guest `shutdown` must terminate QEMU and count as failure. Serial capture is
`-serial file:` (not `stdio`) so the harness reads the full file after the process exits.

## 3. Image recipes (normative layout used by all later phases)

`cargo xtask boot-image 32|64|floppy` writes sparse raw images under `target/`:

- **`boot32.img` / `boot64.img`** — 16 MiB raw HDD:
  - **LBA 0** — MBR: stage1 bootstrap in bytes `0..446` (this plan fills it;
    `rxpart::mbr::encode_mbr` zeroes that area today, `fs/partition/src/mbr.rs:55-69`, so xtask
    stamps `encode_mbr`'s output for the table and then splices stage1 into `0..446` — same raw
    stamping precedent as `tests-integration/src/qemu.rs:328-333`).
    One partition entry: bootable, type `0x06`, **`start_lba = 2048`** (1 MiB — unlike the
    existing boot disks' LBA 63, the widened post-MBR gap is what fits stage2: 63 gives only
    31 KiB, stage2's budget is 160 KiB), count = remainder from 2048.
  - **LBA 1..2047** — gap. **LBA 8** holds the **stage2 image**: `[512 B header][stage2 flat bin]`,
    header = `magic 'ROS2' (u32) | entry_offset (u32) | total_bytes_including_header (u32) |
    reserved (u32)`. Stage1 reads the header sector first, then transfers
    `ceil(total/512)` sectors; the gap never overflows.
  - **LBA 2048..** — FAT16 volume (mtools `-i image@@1048576` offset pattern), label
    `ROSXBOOT`, root contains `KERNEL.ELF` and `INITRD.BIN` (populated from phase 7 on).
- **`floppy.img`** — 1.44 MiB super-floppy (no partition table — the pre-1995 BIOS path): stage1
  in the boot sector, stage2 image at LBA 8, rest padding. **No FAT on the floppy**: the floppy
  path only ever exercises stage1→stage2 raw loading (the workspace engine `rxfat` is FAT12-
  `Unsupported` by design and stays out).

Random `INITRD.BIN` content is generated from a **seed recorded by the packager** so the harness
can recompute the expected CRC32 on the host.

## 4. `cargo xtask boot-test`

Runs the checkpoint suite for the phases implemented so far; each test is declared as
`(name, image, qemu args, timeout, expected_exit, stdout_predicates)` in a table in
`boot_test.rs`, so every later phase only appends rows. Output format mirrors the existing test
culture: one line per checkpoint, full serial log dumped on failure. `cargo xtask test` invokes
the boot-test set unless `--skip-integration`; CI (`ci.yml`) gains a `bootloader` job: build all
bootloader artifacts, run the §6 audits from `00-overview.md`, run `boot-test`.

## 5. Checkpoints (phase 1)

1. Workspace wiring: `ros-boot-common` crate (16550 init/put string/hex, CRC32 table-less,
   `debug_exit(code)` helper centralizing the port `0xF4` law) compiles for both bare-metal
   targets; `xtask boot-test` with an empty checkpoint table exits 0.
2. Harness self-test: a trivial `.code16` stub that prints `BOOTLOADER-HARNESS OK` and writes
   `0x10` to `0xF4`, built by the same toolchain path phase 2 will use (`as` with `.code16`,
   `ld -Ttext 0x7C00 --oformat binary`; binutils presence check fails with a clear message when
   a tool is missing). Three rows: pass (exit 33 + stdout), deliberate fail stub writing `0x01`
   (exit 3), deliberate crash (`ud2`-style under `-no-reboot`, exit 1 classified as
   "crashed before verdict") — plus a timeout row. This is where the exit-code law gets tested
   before any real stage depends on it.
3. `boot-image` produces the MBR/gap/FAT16 layout with a placeholder stage1 blob; `mdir` lists
   the volume at offset 2048 (proof the recipe works before any real stage exists).

**Pass criteria:** all three rows green twice in a row; `cargo xtask test --skip-integration`
unaffected; existing suites untouched.
