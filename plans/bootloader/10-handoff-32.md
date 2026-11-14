# Phase 10 — 32-bit No-MMU Handoff + `fake-kernel-32` (full form)

The 32-bit story completes: `BootInfo` is assembled per the frozen ABI, the kernel is entered by
register-magic convention, and the fake kernel independently re-verifies everything through
pointers — the first end-to-end proof that the treaty is real.

## 1. Stage-2 side

- Finalize the memory map exactly to the reservation table (`00-boot-abi.md` §5), then write
  `BootInfo` (16-byte aligned) at the top of the `0x30000` window **last, after every disk read
  is done** — map array sits immediately below it in the same window; the write ordering is the
  invariant (bounce traffic can no longer clobber it because the bounce lives at `0x40000`, but
  the "write-BootInfo-last" rule is kept anyway: it survives future reordering mistakes).
- `memory_map_ptr`/`initrd_addr` at their **physical** values (no conversion on the 32-bit path).
- Entry check: assert `CR0.PG == 0` (it must still be — nothing enabled it), PIC untouched,
  IF=0 (never set since stage1's `cli` — state it, don't trust it: read `EFLAGS`).
- Handoff: `ESP` = 16-byte-aligned top of the reserved handoff stack (`0x0007_F000` region),
  `EAX = BOOT_INFO_MAGIC`, `EBX = &BootInfo`, `jmp kernel_entry` (never `call`).

## 2. `fake-kernel-32` full form

`extern "C" fn(eax: u32, ebx: *const BootInfo) -> !`:

1. Assert `eax == BOOT_INFO_MAGIC` before dereferencing anything (register-magic discipline);
   then `magic`/`version == BOOT_INFO_VERSION`/`arch == X86_32`/`boot_method == Bios` from the
   struct.
2. Walk the memory map through `memory_map_ptr` with `memory_map_entry_size` stride: assert
   sorted, non-overlapping, `Usable` total sane against the printed stage2 values; count lines
   match `memory_map_len`.
3. Slice `initrd_addr..initrd_size` (physical, paging off), recompute CRC32 **independently**.
4. Emit exactly:
   ```text
   kernel loaded
   arch: x86_32
   initrd_crc32: 0xXXXXXXXX
   ```
   then `0x10` to port `0xF4`. A `jmp`-entered kernel that returns is a contract violation — the
   crate's epilogue writes `0x01` (exit 3), it never returns normally.
5. Decoding goes through `system::boot_info`'s own accessors — the fake kernel is the ABI's
   second consumer and must eat the treaty dogfood, not hand-assume offsets (`size_of`/offset
   unit tests in `system` are the single source of truth).

## 3. Verification

1. `handoff32` on `boot32.img` (`-cpu 486`, `-m 32M`): stdout contains `[INITRD] … crc32=X`,
   then `kernel loaded`, `arch: x86_32`, `initrd_crc32: X` with **X identical to stage2's log and
   to the host-seeded expectation**; exit **33**.
2. `handoff32-512m`: same at `-m 512M`.
3. `handoff32-no-initrd`: packager variant with `INITRD.BIN` absent → `BootInfo.initrd_size == 0`
   path: fake kernel prints `kernel loaded` + `initrd_crc32: 0x00000000`, exit 33 (the
   `size == 0 ⇒ addr == 0` contract row).
4. The three-way CRC assertion is the phase's headline gate — host == stage2 == kernel.
5. All prior rows + audits green.

**Pass criteria:** rows 1–5 green ≥ 3 consecutive `boot-test` runs; the 32-bit boot path is now
functionally complete and this is the natural checkpoint to post progress for review before the
long-mode phases; commit.
