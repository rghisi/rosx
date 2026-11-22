# ROM as initrd — the kernel image sheds its user space

> **Status:** Proposed. Written 2026-11-22 on branch `ccr-1c338027-ychzpi` at `origin/main` =
> `5b56908` (#196). Nothing here has been executed.
> **Goal:** the kernel ELF carries no user-space bytes. The ROM image (and with it `init`)
> arrives as the bootloader's **initrd** — `BootInfo.initrd_addr/initrd_size`, already placed,
> memory-carved by stage2 on both arches — and the kernel loads `init` out of it.
> This is the "rom.img as initrd" item (decision D5/D7) that `plans/x86_32-carrier-swap.md`
> and `plans/x86_64-carrier-swap.md` left open, and the last open §9 stub of
> `plans/bootloader/00-overview.md` besides era hardware.
> **Method:** every code fact is cited to a path re-verified against this tree; facts the plan
> cannot prove at authoring time are marked **unverified** and routed to the step that must
> prove them.
> Repo law applies throughout: `system::boot_info` and the frozen handoff contract
> (`plans/bootloader/00-boot-abi.md` §3) are consumed unchanged; no comments outside the frozen
> baseline (the one allowed shape: a ≤ 6-line `///` linking its rationale on an item containing
> `unsafe`); commits bare imperative; every step lands green through `cargo xtask check`; the
> series ends with `cargo xtask verify` on a quiet host.

---

## 0. Goal & Non-Goals

### Goal
- `arch/x86_64/src/main.rs` and `arch/x86_32/src/main.rs` stop `include_bytes!`-ing `rom.img` and
  `init`. The kernel reads the initrd slice from `BootInfo`, registers it with
  `kernel::rom::init`, locates `init` inside it with `system::rom::RomImage`, and loads it with
  the unchanged `Kernel::load_elf_program`.
- `init` joins the ROM roster (`BOOTSTRAP_APPS`, 8 → 9 names). Init remains the ROM's reader
  through the unchanged opaque `RomRead` syscall.
- The boot disk and every harness master carry the ROM as **`::/INITRD.BIN` at the volume root**
  (stage2 looks only there: `arch/x86-bootloader/stage2/src/fat.rs:14`; deeper paths OOM its
  bump heap — `docs/todo/stage2-bump-heap-never-frees.md`). `/boot/rom.img` and the `/boot`
  directory leave the production disk.
- The kernel build no longer depends on the user-space build: both `build.rs` guards and
  `rerun-if-changed` lists for `rom.img`/`init`/app sources are deleted, and the apps → ROM →
  kernel ordering constraint in `xtask` and CI is lifted (ordering is still kept where the
  *disk* needs all three).
- A missing, empty or malformed initrd is a loud kernel panic with a named reason, not a silent
  empty ROM.

### Non-Goals
- **No init-name parameter.** Passing the init program's name to the kernel is a recorded
  future (user decision); this plan hard-codes `"init"` in one kernel constant.
- **No `BootInfo` change.** `initrd_addr/initrd_size` already carry what is needed
  (`system/src/boot_info.rs:83-84`; ABI row `plans/bootloader/00-boot-abi.md:113`); no CRC
  field is added (that is an ABI change — stop-and-ask territory).
- **No stage2 functional change beyond dropping its CRC32** (D6) — unless R1 fires and proves a heap fix necessary.
- **No `rom:` → `initrd:` rename.** Token grammar, `ROM_SOURCE_PREFIX`, `RomRead`, `kernel::rom`
  and the `rom` wording stay; "rom" is now simply the initrd's content format. A rename is a
  separate cosmetic task.
- **No VFS exposure.** The ROM is still never a VFS volume; `init` is still its only user-space
  reader (`docs/architecture/kstatsd-and-rom.md`).
- **No era-hardware claim** (AGENTS.md hardware-evidence rule); the QEMU timing note in §4 R2 is
  QEMU-only evidence.

---

## 1. Verified baseline facts

All re-verified against `5b56908`.

- **Today the kernel embeds both blobs.** `arch/x86_64/src/main.rs:70-73` and
  `arch/x86_32/src/main.rs:54-57`: `static ROM_IMAGE = include_bytes!(…/rom.img)`,
  `kernel::rom::init(ROM_IMAGE)`, `static INIT_ELF = include_bytes!(…/release/init)`, then
  `kernel.load_elf_program("init", INIT_ELF, boot_plan.encode(), FutureOwner::Kernel)`.
  `arch/x86_32/build.rs:10-20` and `arch/x86_64/build.rs:15-45` panic when the files are absent
  and `rerun-if-changed` them (x86_64 also lists `randomd`/`echo`/`pong`/`vfsd`/`shell` sources
  and ELFs — leftovers of the pre-ROM era).
- **The channel is already built, tested and unused by the kernel.** `stage2` streams
  `/INITRD.BIN` to `kernel_phys_end` page-aligned, prints `[INITRD] addr=… size=… crc32=…` (the CRC dies in S2a, D6),
  carves `[kernel_phys_start .. initrd_end]` (x86_64: `.. page_tables_end`) as
  `KernelAndModules`, and writes `initrd_addr/initrd_size` into `BootInfo`
  (`stage2/src/fat.rs:27-80`, `stage2/src/main.rs:105-140`). x86_64 adds
  `HIGHER_HALF_OFFSET` to a non-zero address (`common/src/bootinfo.rs:46-57`). A missing file is
  not an error: `place_initrd` returns `addr = 0, size = 0` (`fat.rs:35-45`) — the case the
  kernel must now refuse.
- **ABI row** (`00-boot-abi.md:113`): "`size == 0 ⇒ addr == 0`"; same address-space rule as
  `memory_map_ptr` (physical on x86_32, higher-half direct map on x86_64, where stage2's tables
  identity-map `0..4 GiB` and alias the higher half — `plans/x86_64-carrier-swap.md` §1).
- **The kernel's BootInfo consumer reads only the memory map.**
  `kernel/src/platform/bootinfo.rs::usable_blocks` validates magic/version/arch/entry size/
  map length/map pointer/VGA display with width-exact reads at `offset_of!` positions and no
  enum decode except `MemoryKind::from_wire`; its design doc is
  `docs/rationale/x86-bootinfo-consumer.md`. It has host unit tests over a fabricated window
  (`MAP_OFFSET`, `INFO_OFFSET`, `region`, `header`, `stage2_header` helpers).
- **The ROM codec lives in `system`.** `system/src/rom.rs`: `ROM_MAGIC`, `RomImage::parse`
  (validates every entry: names, duplicates, `offset+len ≤ len` in `u64`), `encode`. `kernel`
  may depend on `system` (AGENTS.md). `kernel/src/rom.rs` is today a codec-free opaque blob
  (`Once<&'static [u8]>`, `read_at`) because of `plans/rom-filesystem-vfs.md` LD4; that law is
  retired here, deliberately (D2).
- **`load_elf_program` is eager and retains nothing** (`kernel/src/kernel.rs:228`;
  `docs/architecture/memory.md`: "the kernel retains no ELF bytes after a load"), so loading
  `init` from initrd memory does not pin it; the initrd region stays valid as `RomRead` backing
  for the whole boot because the memory map reserves it.
- **Packaging today.** `xtask/src/main.rs::pack_rom` encodes `spec.rom_apps`
  (`BOOTSTRAP_APPS`, `xtask/src/arch.rs:7`, `[&str; 8]`, sorted) to
  `target/<user_dir>/rom.img`; `xtask/src/boot_disk.rs:104` copies it to `::/boot/rom.img`,
  which nothing reads. `boot_image.rs:29-38,352` writes a seeded 64 KiB synthetic
  `INITRD.BIN` for the fake-kernel media only (`INITRD_VOLUME_NAME` is already `pub`).
- **Harness today.** `tests-integration/src/fat_disk.rs::create_boot_master` copies every built
  user ELF plus `KERNEL.ELF`; `copy_built_els`/`built_els` filter `!name.contains('.')`, so
  `rom.img` is *not* copied; `tests-integration/tests/disk_boot.rs::build_boot_disk` builds its
  own media the same way. Integration runs prebuilt release kernels via `ROSX_KERNEL_ELF`
  (AGENTS.md) — so after this plan the ROM travels **only** through the harness disk.
- **Real-kernel boot-test rows** (`xtask/src/boot_test.rs:435,452`) boot the built unified disks
  and require kernel banners; they SKIP loudly when the disk is absent or not a chain disk
  (`carries_boot_chain`, 574).
- **CI** (`.github/workflows/ci.yml:53-101,124-152`, `build-artifacts.yml`): build jobs run
  `cargo xtask build` (apps + ROM + kernel + disk) and upload the disks; the x86_32 job also
  uploads `rosx-x86_32` + `rom.img` as separate artifacts; integration jobs run
  `cargo xtask build` first, then the suite with a prebuilt kernel.
- **stage2 CRC32 exists only to prove the bootloader read the payload.** It is a bitwise
  table-less loop (`common/src/crc32.rs:3-14`, 8 iterations per byte) over the in-memory initrd
  (`fat.rs:43,78`), printed in `[INITRD]`; consumers are `boot_test.rs:866-910` (host seed ==
  stage2 banner == fake-kernel recompute) and the two fake kernels, which recompute
  independently (`fake-kernel-{32,64}/src/main.rs`). With a real ROM it would run over the
  full image on every boot — cost **unverified**, and pure overhead once the kernel's own
  parse is the real proof.
- **Unsafe ledger.** `check-style` baselines `unsafe` per crate in `xtask/style-baseline.txt`;
  raising a count requires a decision record in the same commit
  (`check-style --update-baseline --allow-increase "<reason>"`).

---

## 2. Decisions

- **D1 — Transport = the existing `INITRD.BIN` at the volume root; ROM format unchanged.** The
  initrd *is* `rom.img` byte for byte (`ROSXROM1` container). No wrapper, no second format.
- **D2 — The kernel parses the ROM (LD4 retired).** The kernel needs one fact from inside the
  ROM — the `init` ELF — so `kernel::rom` gains `program(name) -> Option<&'static [u8]>` over
  `system::rom::RomImage::parse` (allocates a small entry `Vec`; the heap is up by then).
  `RomRead` and the opaque `Once<&'static [u8]>` stay exactly as they are; the codec is
  `system`'s, so no format knowledge is duplicated (architecture principle: the shared treaty
  holds layouts).
- **D3 — Kernel is the single authority on initrd validity, validated once at the edge.** A new
  `unsafe fn initrd(address, expect_arch, regions…)` in `kernel/src/platform/bootinfo.rs`
  (beside `usable_blocks`, same width-exact-read discipline) returns
  `Result<Option<&'static [u8]>, BootInfoError>`:
  - `size == 0 ⇒ addr == 0 ⇒ None`; `size == 0 ⊕ addr == 0` is a `BootInfoError` (ABI row).
  - address and `address + size` must fit `usize` without overflow (x86_32 32-bit `usize`).
  - the whole range must lie inside one `MemoryKind::KernelAndModules` region of the
    published map; this is the property that makes the allocator-never-touches-the-ROM claim
    checkable rather than assumed. On x86_64 the check compares the direct-map address minus
    `HIGHER_HALF_OFFSET` against the physical map.
  The caller (arch `main`) turns `None` into a panic naming the cause.
- **D4 — Failure policy for the ROM is panic, not log-and-continue.** Boot-plan failure policy
  (log-and-continue) is about *steps inside init*; without a ROM there is no init and no
  console stack, so the kernel panics with one of: `no initrd (INITRD.BIN missing on the boot
  volume)`, `initrd refused: <BootInfoError>`, `initrd is not a ROM image: <RomError>`,
  `initrd carries no init`. Mirrors today's `unknown boot magic` and `init ELF rejected`
  panics. A `[ROM] addr=… size=… files=…` line is printed on success and is the integration/
  boot-test anchor.
- **D5 — `init` rides `BOOTSTRAP_APPS`.** Same class as the other members: programs needed
  before `/` exists. `rom_apps` keeps its per-arch field; both arches still pack the same set.
  `/init` stays in `USER_APPS` (built, copied to the disk root as today); only the kernel's
  copy moves.
- **D6 — stage2 drops its CRC32 (user decision).** The CRC existed to prove the bootloader
  really read the images. The proof is kept where it is independent: the fake kernels still
  recompute CRC32 over the initrd they receive and print `initrd_crc32:`, and `boot-test`
  asserts that equals the host-seeded payload's CRC (two-way: host == kernel view). Stage2
  prints `[INITRD] addr=… size=…` only; `fat.rs` loses its `crc32` calls and the `report`
  checksum parameter; the `boot_test.rs` stage2-banner-CRC leg goes. `crc32` stays in
  `rosx-boot-common` (its remaining users are the fake kernels and `boot_test`'s host side).
  The real kernel's proof is behavioural: the D3 range check, `RomImage::parse`, and `init`
  loading. No `BootInfo` field, no ABI change.
- **D7 — One constant, one place.** `INITRD_VOLUME_NAME` (`boot_image.rs`) is the single name
  for the root file; `boot_disk.rs` and the harness reuse it (the harness crate cannot depend
  on xtask — it mirrors the constant behind the same `CHAIN_KERNEL_VOLUME_NAME` pattern it
  already uses for `KERNEL.ELF`).

---

## 3. Steps

Each step is green through `cargo xtask check`, committed bare-imperative with the repo's
attribution trailer, pushed to the working branch, and ends with the `retro` skill's step-end
mode reported with the step's commit. No PR unless asked.

### S0 — Baseline and measurements (no code)
- Quiet-host gate (`pgrep -f 'cargo test|xtask test|qemu-system'`, load vs `nproc`) via a
  script file (its own cmdline must not match — `plans/integration-speedup-impl.md` step 0).
- Build both arches (`cargo xtask build`, `cargo xtask build x86_32`; logs to files with exit
  codes) and record: `rom.img` size per arch (today's 8 apps — **unverified**, estimated a few
  hundred KiB), `KERNEL.ELF` size per arch, and one `[STAGE2_*]`-to-`[KERNEL] Starting` boot
  wall per arch with the unchanged tree. These are the "before" numbers for S5.
- Confirm stage2 heap headroom with today's production media (the todo's measured 55,552 /
  65,536 on stage-64 was sparse test media). Commit nothing; the record goes into the S5
  execution note.

### S1 — Kernel consumer (`kernel/`, host-tested; no behaviour change yet)
- `kernel/src/platform/bootinfo.rs`: add `initrd` (D3) with `BootInfoError` variants
  (`Initrd`, `InitrdRange`, `InitrdNotReserved`), its ≤ 6-line `///` linking
  `docs/rationale/x86-bootinfo-consumer.md`, and extend that rationale (the one new
  `from_raw_parts` over the initrd bytes: caller contract = producer keeps them readable and
  the map reserves them; what breaks if loosened = the allocator hands out the ROM).
- `kernel/src/rom.rs`: add `program(name)` (D2) over `RomImage::parse(image())` and a
  `validate(image) -> Result<usize /*files*/, RomError>` helper for the `[ROM]` line, with
  host tests (hit, miss, malformed, empty, init absent).
- Host tests for `initrd`: absent (0/0), half-zero, overflow, range straddling a `Usable`
  region, range inside `KernelAndModules`, x86_64 higher-half translation, wrong arch/magic
  (reusing the existing fabricated-window helpers).
- Ratchet: `unsafe` for `kernel` rises by one → decision record
  `docs/decisions/gate-baseline-unsafe-raised-2026-11-22.md` written by
  `check-style --update-baseline --allow-increase "bootinfo initrd slice"`.
- Proof: `cargo xtask check`; `cargo test -p kernel -- --test-threads=1`. Nothing boots
  differently.

### S2 — Drop stage2's CRC, then the data plane: the disk carries `INITRD.BIN`
Two commits (each green): **S2a** the CRC removal, **S2b** the data plane.

**S2a — stage2 without CRC32 (D6)**
- `stage2/src/fat.rs`: remove the `crc32` import/calls and the `report` checksum argument and
  the ` crc32=` print; `[INITRD] addr=… size=…` stays (also for the absent case).
- `xtask/src/boot_test.rs`: drop the stage2-banner CRC parse/compare (`~866-900`); keep the
  host-seed == fake-kernel `initrd_crc32:` comparison (`~894-910`); adjust any row expectation
  matching `crc32=` text. `handoff32-no-initrd` keeps its empty-payload
  `initrd_crc32: 0x00000000` from the fake kernel.
- Docs: `arch/x86-bootloader/README.md` (lines 38, 42, 140-141, 156: "three ways" →
  "two ways, host seed against the kernel's recompute"); `plans/bootloader/09-initrd-crc.md`
  gets a short superseded note; `arch/x86-86box/README.md:124` re-read (it names the fake
  kernel's `initrd_crc32`, so likely untouched).
- Proof: `cargo xtask check`; `cargo xtask boot-test` all rows PASS with the two-way check; a
  deliberate-failure proof that the check still bites (perturb the host seed, expect the
  comparison row to fail); `check-isa bootloader`; stage2 size/heap margins re-read (they
  should only shrink).

**S2b — data plane** (kernel still embeds; both paths coexist)
- `xtask/src/arch.rs`: `BOOTSTRAP_APPS` gains `"init"` (`[&str; 9]`, D5).
- `xtask/src/boot_disk.rs::fill_volume`: copy `spec.rom_path()` to
  `::/{INITRD_VOLUME_NAME}`; delete the `::/boot` directory creation and the
  `/boot/rom.img` copy (and make `INITRD_VOLUME_NAME` reachable from there).
- `tests-integration/src/fat_disk.rs`: `create_boot_master` and a small
  `FatDisk::install_initrd` copy `<user_dir>/rom.img` to `::INITRD.BIN`; `disk_boot.rs`'s
  `build_boot_disk` (and any other bespoke media builder — grep `create_bootable(`) call it.
  The harness must fail loudly, by name, when `rom.img` is absent (house law: "run `cargo xtask
  build`").
- Inert for the kernel: it still embeds and ignores `INITRD.BIN`, so stage2 now really loads a
  ROM-sized initrd on every production boot while behaviour is otherwise unchanged. That makes
  S2 the isolated proof of R1 (stage2 heap with a real payload) **before** the
  kernel depends on it.
- Proof: `cargo xtask check`; `cargo xtask build` both arches; both real-kernel boot-test rows
  show `[INITRD] addr=… size=<rom size>` and still reach `rose>`; the production disk
  `mdir` lists `INITRD.BIN` and no `boot`; full integration suites unchanged.

### S3 — The flip: kernel boots from the initrd (one commit, both arches)
The two arch mains, both `build.rs` files and the boot-test banner move together — between them
the tree does not boot.
- `arch/x86_{64,32}/src/main.rs`: after `kernel.setup()` obtain the initrd (`bootinfo::initrd`
  with the same `loader_info`/arch/`kernel_phys_end` plumbing `boot_info_memory` already has —
  factor one helper per arch returning the slice), panic per D4, print
  `[ROM] addr=… size=… files=N`, `kernel::rom::init(slice)`, fetch
  `kernel::rom::program("init")`, load it. The two `include_bytes!` and `static INIT_ELF` /
  `ROM_IMAGE` die. Boot-plan body unchanged.
- `arch/x86_64/build.rs`, `arch/x86_32/build.rs`: delete the ELF/ROM asserts and every
  `rerun-if-changed` for user sources/ELFs/`rom.img`; keep only linker-script wiring.
- `xtask/src/boot_test.rs`: the two real-kernel rows require `[ROM] ` and forbid
  `no initrd` / `initrd refused` / `initrd is not a ROM image` / `initrd carries no init`.
- Integration: `disk_boot.rs` asserts `[ROM] ` early in the ROM-fallback test; add
  `a_disk_without_an_initrd_panics_naming_the_missing_rom` (arch-agnostic, builds a master
  *without* `INITRD.BIN`, expects the D4 text and no `rose>`); confirm the existing
  ROM-shell-fallback test still lands on `rom:shell`.
- Proof: `cargo xtask check`; `cargo xtask boot-test` (26/26 + the new banners); full
  integration on both arches (40/40 each baseline — S0 records the current counts);
  `cargo xtask run` both arches reach `rose>`; `readelf`/`size` audit: the kernel ELF shrank by
  at least the S0 ROM size plus init's size.

### S4 — Build-flow simplification
- `xtask/src/main.rs`: `build_kernel` no longer needs apps/ROM first. In `check`'s x86_32 build
  surface (`gate_stages`, ~line 222) drop `build_apps`/`pack_rom` unless `check_isa` itself
  needs a user build (verify, do not assume); keep them where the *disk* is produced
  (`build`, `run`, `image`, `verify`, `run_tests`). Remove the now-impossible "ROM missing"
  guard wording from messages.
- `.github/workflows/ci.yml`, `build-artifacts.yml`: the lone "Upload kernel and ROM" artifact
  (`rosx-x86_32`: kernel ELF + `rom.img`) is no longer a bootable pair on its own; keep both
  files (they are still the only separately downloadable parts) but re-title the step and make
  the unified disk artifact the documented boot artifact on both arches. No job loses a step it
  needs: integration jobs still run `cargo xtask build` first because the harness needs
  `rom.img`, the user ELFs and the staged chain bins.
- Proof: `cargo xtask check`; a clean-tree `cargo xtask build x86_32` order test — touching an
  app source must now rebuild the ROM/disk but **not** relink the kernel (`ls -l` mtimes of
  the kernel ELF before/after; this is the speed-up the change buys).

### S5 — Docs and records
- `AGENTS.md`: the `kstatsd`/ROM invariant line — the ROM arrives as the bootloader initrd and
  both ports pack `BOOTSTRAP_APPS` (now incl. `init`); the kernel loads `init` from it.
- `docs/architecture/kstatsd-and-rom.md` (invariants + "the kernel's own copy of init is a
  separate `include_bytes!`" sentence, rewritten), `boot-media-and-harness.md` (line 15 `/boot/
  rom.img` → `INITRD.BIN`; line 21 "The kernel embeds the init ELF and the ROM image" → the
  initrd story; harness paragraph: masters carry `INITRD.BIN`; new bullet in Invariants),
  `boot-plan.md` (ROM bootstrap sentence), `docs/architecture/memory.md` if its ROM wording
  names embedding, `arch/x86-bootloader/README.md:18` (rom.img-as-initrd now done),
  `plans/bootloader/00-overview.md` §9 (close the stub), `plans/rom-filesystem-vfs.md` (LD4
  superseded note), README.
- Decision records: `docs/decisions/kernel-parses-the-initrd-rom.md` (D2, LD4 supersession,
  `status: decided`), the unsafe-baseline record from S1 (already filed).
- `docs/todo/`: `init-program-name-as-a-kernel-parameter.md` (user's future item; trigger:
  a second program wants to be init, or a command-line/BootInfo extension exists) (no CRC todo: removed). Existing
  `rom-app-names-revert-underscore-free.md` untouched.
- Proof: `cargo xtask check`; the docs-verification pass the repo already does for
  architecture docs (`docs/decisions/architecture-docs-verification-findings.md`) re-run on the
  touched files.

### V — Merge evidence
- Quiet-host check, then `cargo xtask verify` captured to a file with its exit code and the
  chain's start/end stamps (AGENTS.md verdict law); verdict read from the files.
- Deliberate-failure proofs (the carrier-swap law): (1) delete `INITRD.BIN` from the built
  production disk → kernel panics with `no initrd`, boot-test-style timed-out verdict, disk
  restored bit-identical; (2) overwrite one byte of the ROM magic in `INITRD.BIN` → panic
  `not a ROM image`.
- Retro in full-workflow mode with the final report.

---

## 4. Risks and contingencies

- **R1 — stage2 heap with a real, large initrd.** `Engine::open` + a long cluster-chain read add
  allocations on top of the 55,552 / 65,536 stage-64 peak (`docs/todo/stage2-bump-heap-never-
  frees.md`, whose trigger names "a bigger read window" and x86_64 work). The kernel ELF already
  streams through the same engine, which bounds the risk, but the *combination* is unmeasured.
  S2 isolates it: if `[ERR] heap` appears, the todo's cure (LIFO tail-free bump) lands as its own
  commit before S3 — a stage2 change without ABI impact, and the one authorised exception to
  "no stage2 change".
- **R2 — (retired by D6).** The bitwise CRC over a ~1 MiB ROM was the one boot-time cost this
  change would have added on 486-class CPUs; it no longer exists. S0/S2 still record the
  `[INITRD]`-to-next-banner delta in QEMU as before/after evidence, labelled QEMU-only.
- **R3 — Memory floor and map arithmetic.** The initrd lives right after the kernel and before
  the x86_64 page tables; with a larger payload the `place_payload` refusal (`[ERR] mem`) can
  fire on small `-m`. Current harness/boot-test machines are ≥ 32 MiB (`-m 32M` rows exist for
  stage2); a ~1 MiB ROM is nowhere near, but the 486-class RAM figure of real targets is the
  hardware checklist's (`docs/architecture/real-hardware-x86_32.md`), not claimed here.
- **R4 — x86_32 `usize` and the D3 range check.** 32-bit `usize` makes `address + size`
  overflow checks load-bearing; covered by S1 host tests built for both widths (the test crate
  compiles on the host, so the 32-bit case is exercised by value, not by target — note the
  limitation; the QEMU x86_32 boot is the real proof).
- **R5 — Stale ROM vs fresh kernel in harness.** Because the kernel no longer embeds the ROM, a
  harness master built from an old `rom.img` boots a new kernel without complaint. Mitigation
  is structural: `run_tests`/`verify` always `pack_rom` before the suite (as today) and the
  harness reads `rom.img` at master-build time; no cache of ROM bytes is introduced.
- **R6 — Flakiness.** Integration evidence obeys the quiet-host rule; a contention failure is not
  a verdict (`plans/integration-flakiness-assessment-2026-11-20.md`).

## 5. What closes / what stays open

Closes: decision D5/D7 "rom.img as initrd" of both carrier-swap plans; §9 *rom.img as initrd*
of `plans/bootloader/00-overview.md`; the `build.rs` coupling of kernel to user space.
Stays open: era-hardware verification (both arches); the init-name-as-parameter item (recorded
as a todo); `rom:` → `initrd:` naming; VESA framebuffer successor.

---

## Friction log

(empty — filled during execution, one line per friction: `step | what | class | cost`)
