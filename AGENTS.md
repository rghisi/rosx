# RosX - Agent Context Document

> **Last Updated:** 2026-11-09
> **Purpose:** Compact, always-accurate context for AI coding agents working on RosX.

---

## Project Overview

**RosX** is a learning/hobby OS project written in Rust, aiming to be a multi-platform operating system capable of running on diverse hardware - from microcontrollers to m68k computers to x86_64 systems. The goal is to create a multi-purpose OS that can run network applications such as web servers.

### Key Characteristics
- **Language:** Rust (with minimal assembly for architecture-specific code)
- **Target Platforms (status):**
  - **x86_64** — bootstrap platform, complete.
  - **x86_32** — complete (Multiboot v1/GRUB, `arch/x86_32/`); 486-class hardware is a maintained baseline.
  - **ARM, RISC-V, m68k** — future (no `arch/` dirs exist yet; m68k is feasibility-stage only).
- **Scope:** Full-featured OS — scheduling, interrupts, IPC, ELF user-space; networking is a goal, not yet implemented.
- **Project Type:** Learning/hobby project with practical goals

**Platform Strategy:** x86_64 is the starting point due to tooling, not the focus — keep `kernel/` portable.

---

## Project Structure

```
rosx/                        # Cargo workspace (edition 2024, nightly)
├── kernel/                  # Platform-agnostic core (see submodules below)
│    scheduler/ (SchedulingAlgorithm: MLFQ + FIFO)
│    memory/   (global allocator; FreeList + BitmapChunk)
│    collections/ (arena)  handle/ (5 newtypes: task/future/ipc-connection/ipc-binding/ipc-data)
│    elf/ ipc/ (mailbox, data) future/ syscall/ kstatsd task*  platform/ (cpu, elf, interrupt, output, portio HAL traits + per-task port grants, acquired exclusively from the arch pool)  kconfig  rom/
├── system/      # ABI treaty: syscall numbers, wire layout, ROM image layout, boot plan grammar, error numbers
├── usrlib/      # User-space libc: allocator (binned slabs, global default), syscall, exit, out, arch/{x86_64,x86_32}
├── protocol/                # Service RPC treaties, one sub-crate each (workspace glob protocol/*)
│    vfs/         # package vfs-protocol: wire codec + typed client
│    input/       # package input-protocol: wire codec
│    ata/         # package ata-protocol: wire codec + typed client
│    block/       # package block-protocol: generic block-device family (reserved opcode band) + manager contract (blockd namespace) — wire codecs + dial-by-name clients
├── fs/
│    core/        # engine treaty (trait Engine + device treaty BlockDevice + FsError/Kind/DirItem/MetadataView), no deps, shared by rxfs, rxfat, ramfs and vfsd
│    rxfs/        # no_std filesystem library (vfsd's engine): superblock/bitmaps/inodes/extents/dirs, leak-safe sync, check()
│    fat/         # package rxfat: no_std FAT16/FAT32 read/write engine (8.3 names, total boot-sector parser), vfsd's second mountable engine
│    partition/   # package rxpart: total MBR parser, curation rule and encoder, blockd's exposure oracle
│    ramfs/       # no_std in-memory filesystem engine, rxfs-shaped (lookup/open/create/mkdir/rmdir/read/write/close/remove/readdir/stat) over a path-keyed store
├── arch/
│    x86_64/        # entry, cpu, interrupts, framebuffer, terminal_fonts, *.S (global_asm)
│    x86_32/        # entry, cpu, interrupts, boot.S (Multiboot v1)
│    x86-generic/   # shared x86-family mechanism: port executor, VGA text buffer, debug console, ANSI parser, Multiboot v1 memory decode
│    x86-runner/    # workspace member: kernel → boot medium + QEMU launcher for both ports (NOT bootimage)
│    x86-86box/     # data-and-scripts (no crate): era-VM template, GRUB Legacy media recipes, provenance manifest, runbook
└── apps/
     [workspace members]  shell, init, dummy, pi, randomd, echo, bulk, pong, ping, mem, ps, vfsd, vfs, snake, tetris, conway, threads, waitset, stress, inputd, ps2d, atad, ata, blockd, bench, format, mount, attach, rm, cp, mkdir, rmdir, exiter
```

---

## Build & Run

Canonical recipes: `.github/workflows/ci.yml` (build + test) and `build-artifacts.yml` (disk images).

### x86_64
```bash
cargo xtask build              # ELF apps + ROM image + kernel + BIOS boot image + FAT boot disk
cargo xtask apps               # ELF apps only
cargo xtask rom                # ROM image from built apps
cargo xtask test               # all host unit tests
cargo build -p rosx            # kernel only (from arch/x86_64 directory or with target)
cargo run -p rosx              # run.sh -> arch/x86-runner -> BiosBoot image + FAT disk -> QEMU
```
- `cargo run` uses the **custom runner** (`.cargo/config.toml` → `./run.sh`), which invokes `arch/x86-runner` to make a BIOS disk image with the `bootloader` crate's `BiosBoot` (attached on the **secondary** IDE bus with `bootindex`, so the kernel is still loaded via BIOS but the carrier never enters the guest namespace), then starts `qemu-system-x86_64`. **Not bootimage.** The `bootloader` crate's stages honor the BIOS-supplied drive number end to end, so the carrier works from any IDE slot.
- **FAT boot disk** — `cargo xtask build` (x86_64) also writes `target/rosx-boot-disk.img` (host `mformat`/`mmd`/`mcopy` from mtools), the same recipe as x86_32: a 64 MiB image whose MBR (stamped with `rxpart::mbr::encode_mbr`) carries one bootable FAT16 slice (type 0x06, LBA 63) with every user ELF at the root plus `/boot/kernel.elf` and `/boot/rom.img`. Both `cargo run` (when the image is present) and `cargo xtask run` attach it at `if=ide,index=0`, so the curated namespace is `ata0d0` with slice `ata0d0p0`; the on-disk `kernel.elf`/`rom.img` are forward-looking artifacts for a real GRUB boot, not mechanisms (the kernel uses its embedded ROM and its BIOS carrier).
- Both `cargo run` and `cargo xtask run` attach a persistent host-side 50 MiB IDE disk at `if=ide,index=1` (`$HOME/tmp/rosx-hdd.img`, created if missing, never truncated, survives VM restarts): `attach ATA` → `format ata0d1` → `mkdir /<dir>` → `mount ata0d1 /<dir>` → `ls /<dir>` (the mount point is a real root-volume directory that must exist before mounting; `attach ATA` registers the ATA controller with `blockd` — the single block-device provider whose namespace lives in memory, so every boot re-attaches; the persistent disk `ata0d1` has no partition table, so it stays a whole drive). Because the kernel carrier rides the secondary bus (outside atad's primary-channel port pool), the boot disk is the FAT volume `ata0d0`/`ata0d0p0` and the old `ata0d0p1`/`ata0d0p2` bootloader-slice footgun is gone. The headless integration harness keeps its own tempdir disks (`QemuSession::spawn_with_hdd_disk` for the persisting flow; `spawn_with_partitioned_hdd_disk` / `spawn_with_corrupt_partitioned_hdd_disk` hand-carve an MBR into the image first).
- Target spec: `arch/x86_64/rosx.json` — bare-metal `no_std`, `build-std` = core/alloc/compiler_builtins.
- User-space ELF apps (pi, randomd, snake, tetris, conway) are workspace members; build with `cargo xtask apps` or `cargo xtask build`.
- Boot is plan-driven: each arch port embeds only the ROM image, the `init` ELF, and `kstatsd`; `init` walks the arch's `BootPlan` (`system::boot_plan`, passed as init's argv) — services launch from ROM and wait their bind-name probe, `run` steps wait a tool's exit, `exec` hands the console over and init exits. Both ports boot the same 12-step plan (`rom:atad`, `rom:blockd`, `run rom:attach ATA`, `rom:vfsd`, `rom:inputd`, `rom:ps2d`, `run rom:mount ata0d0p0 /`, `service /randomd|/echo|/pong`, `exec /shell`, `exec rom:shell`); `bulk` is not in the plan (the `ipc_bulk` test launches it from the shell, and bulk exits when done).
- The shell is a user ELF executed by init from the mounted FAT root, falling back to the ROM copy when the disk is missing. It keeps five built-ins (`ls clear rose sleep random`) and resolves every other prompt name through VFS, so disk/ROM files execute by name with arguments (`rose> snake fast`).
- `cargo build --workspace` is intentionally unsupported: bare-metal bins and PIE apps cannot compile for the host.

### x86_32
```bash
cargo xtask build x86_32       # ELF apps + ROM image + kernel (Multiboot v1, boot.S) + FAT boot disk
cargo xtask run x86_32         # boot QEMU (-kernel + the FAT boot disk at ide index=0) to the shell prompt
cargo xtask image x86_32       # GRUB bootable ISO (needs grub-mkrescue, xorriso, mtools)
cargo build -p rosx-x86_32     # kernel only (from arch/x86_32/ — its .cargo/config.toml supplies the rosx-i486 custom target)
```
- The kernel embeds the init ELF and the ROM image that `cargo xtask build x86_32` produces under `target/rosx-i486-user/release/`; `arch/x86_32/build.rs` asserts both are present before compiling.
- **FAT boot disk** — `cargo xtask build x86_32` also writes `target/rosx-i486-boot-disk.img` (host `mformat`/`mmd`/`mcopy` from mtools): a 64 MiB image whose MBR (stamped with `rxpart::mbr::encode_mbr`) carries one bootable FAT16 slice (type 0x06, LBA 63) with every user ELF at the root — all app names must fit the 8.3 stem, or the build says so — plus `/boot/kernel.elf` and `/boot/rom.img`. QEMU loads the kernel via `-kernel` and the kernel uses its embedded ROM, so the on-disk `kernel.elf`/`rom.img` are forward-looking artifacts for a real GRUB boot, not mechanisms. `cargo xtask run x86_32` attaches the image at `if=ide,index=0`, so the curated namespace is `ata0d0` with slice `ata0d0p0`; the persistent host disk stays at index=1 (`ata0d1`).
- The boot plan boots from that disk: the stack comes up from ROM, `run rom:mount ata0d0p0 /` root-mounts the FAT volume, `service /randomd|/echo|/pong …` start **from the mounted volume**, and `exec /shell` hands over to the shell **loaded from disk**; when the disk or its binaries are missing the service steps log `not available` and the plan's last step (`exec rom:shell`) lands on the ROM shell instead. The ROM carries only the bootstrap set (`BOOTSTRAP_APPS` in `xtask/src/arch.rs`: atad, blockd, attach, mount, vfsd, inputd, ps2d, shell — the `rom:` tokens plus the console stack) on both ports; x86_64 runs the identical plan on its own `target/rosx-boot-disk.img`.
- Integration tests run on x86_32 with `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1`; the harness boots the kernel ELF directly, and every session on both arches attaches a **full-apps FAT master** (`tests_integration::fat_disk` — rxpart-stamped MBR + mtools, all ELFs found in the user release dir), so sessions disk-boot to the disk shell and resolve every CLI through the root mount; the ata suites keep their second-drive topology on top. `ipc_vfs` and `ata_env_spike` stay x86_64-only, `disk_boot` is x86_32-only (root mount, disk-shell hand-off, ROM fallback, and a test that boots the real `target/rosx-i486-boot-disk.img` when `cargo xtask build` produced it); `kstats_mem` runs unguarded on both arches (the wake-path race is fixed — `plans/x86_32-wake-race.md`).
- Task stacks (`STACK_WORDS` in `kernel/src/task_stack.rs`) are sized from watermark measurements: 8192 words on x86_64, 12288 on x86_32 — the nested `vfsd`→`atad` format chain has a fixed depth of ~35 KiB (much of it the 4 KiB IPC receive chunks). The x86_32 chain measures nearly the same byte depth as 64-bit: the stack is dominated by fixed-size buffers, not register frames.

### Real hardware (x86_32)

Boot recipe: copy `target/rosx-i486/release/rosx-x86_32` (a Multiboot v1 ELF) onto an IDE disk or CF adapter carrying GRUB Legacy 0.97 or GRUB 2, and chain to it — GRUB Legacy: `kernel /boot/rosx` then `boot`; GRUB 2: `multiboot /boot/rosx` then `boot`. This era path is now scripted: `cargo xtask era-image x86_32` produces the GRUB Legacy 0.97-67 FAT16 HDD image (the recipe below, executed literally) and the El Torito ISO, and the emulator-side rehearsal lives in `arch/x86-86box/` (its README is the runbook). Every QEMU x86_32 session already runs `-cpu 486`, and `cargo xtask check-isa x86_32` audits the shipped ELFs; silicon still needs its own evidence. **Hardware verification checklist — every item open until executed on a board; no item may be marked green without hardware evidence:**

1. Memory detection through a v1 bootloader: `[MEMORY]` banner appears after GRUB; on a GRUB build that lacks the mmap buffer, the `mem_lower`/`mem_upper` fallback must still yield a usable heap (`[MEMORY] Total:` on 4–64 MB expected).
2. `PAUSE`-as-NOP: no `#UD` at first `Once` contention during early boot.
3. PIT 100 Hz / 8259A cascade timing on VLB/PCI chipsets: `SYSTEM_TIME_MS` advances, shell echo works.
4. 16-bit IDE PIO on a real drive: atad identify + LBA read (timing margins, IORDY).
5. VGA text console standalone: writes to the unclaimed debug port 0xE9 must stay harmless; VGA is the only output.
6. The `rep bsf` idiom (objdump prints `tzcnt`): executes as `BSF` (F3 ignored) on silicon.
7. 486SX sub-case (only if SX support is ever chosen): boot with no FPU present.

### Unit tests (run on host)
```bash
cargo xtask test               # all workspace unit tests
cargo test                     # default-members only
cargo test -p kernel -- --test-threads=1   # scheduler, memory, elf, ipc, future
cargo test -p rxfs             # filesystem library (add --features std for the FileDevice suites)
cargo test -p fs-core          # engine treaty crate (types + traits, no tests of its own)
cargo test -p ramfs            # in-memory filesystem engine (rxfs-shaped path-keyed store)
```

### Integration tests (run on host against QEMU)
```bash
cargo test -p tests-integration -- --test-threads=1                      # x86_64
ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1      # x86_32
```
- **Always set `ROSX_KERNEL_ELF` to a prebuilt release kernel** (CI does: `target/rosx/release/rosx`, `target/rosx-i486/release/rosx-x86_32`). Unset, the harness builds one itself in the **dev profile** (`ROSX_QEMU_PROFILE` defaults to `dev`), silently inflating suite wall times ~3x and making runs look like a guest regression when the guest is fine (`tests-integration/src/kernel_build.rs`).

### xtask commands
```bash
cargo xtask build [arch] [--debug]  # ELF apps + ROM + kernel + boot artifact; x86_32 also writes the FAT boot disk (needs mtools)
cargo xtask apps [arch]             # Build ELF apps only
cargo xtask rom [arch]              # Pack the ROM image from the built user ELFs
cargo xtask run [arch] [--debug]    # Build and launch QEMU to the shell prompt
cargo xtask image [arch] [--debug]  # flashable artifact: x86_64 BiosBoot disk image, x86_32 GRUB ISO (needs grub-mkrescue, xorriso, mtools)
cargo xtask era-image x86_32        # era boot media for 86Box/hardware: GRUB Legacy 0.97-67 El Torito ISO + FAT16 HDD (needs mtools, sfdisk, xorriso, docker for the grub shell; see arch/x86-86box/README.md)
cargo xtask check-isa x86_32        # objdump audit: kernel + every user ELF must contain only 486-era mnemonics
cargo xtask check-isa --stages [dir] # audit the pinned GRUB Legacy stage binaries (default target/grub-legacy/stages; fetch via arch/x86-86box/scripts/fetch-grub-legacy.sh)
cargo xtask test [--skip-integration] # Run unit tests; --skip-integration excludes the QEMU tests-integration suite
```

## Development Guidelines

### When Working on RosX

1. **Check current state first:**
   - Read git status and recent commits

2. **Understand the architecture layer:**
   - `kernel/` should remain platform-agnostic
   - Architecture-specific code goes in `arch/[platform]/`
   - x86-family-specific but not bit-width-specific code lives in `arch/x86-generic/`; anything else stays in its port
   - Use traits to abstract platform differences

3. **Assembly code:**
   - Keep assembly minimal and well-documented
   - Prefer Rust with inline asm when possible
   - Document register usage and calling conventions

4. **Interrupt handling:**
   - Always platform / arch specific best practices
   - Interrupts currently disabled during critical sections

5. **Testing:**
   - **Write unit tests for all kernel modules** (see `kernel/src/scheduler/mlfq_strategy.rs` as example)
   - Unit tests should be comprehensive and test edge cases
   - Use `#[cfg(test)]` modules within each file
   - Integration testing: Use dummy tasks for scheduler/task testing
   - Use longer delays to observe task switching behavior in QEMU
   - Verify behavior in QEMU and eventually real hardware

6. **Warnings are errors:** `[workspace.lints.rust]` in the root `Cargo.toml` denies the unused family, implicit unsafe ops in `unsafe fn`, private-in-public interfaces, irrefutable `let-else`, function-item casts, and deprecated APIs; every member crate inherits it. Every build surface (both arches, apps, host unit tests) must end at zero warnings — the only expected diagnostic is the toolchain's own `core` future-incompat report, an artifact of `-Zbuild-std` compiling rust-src and not this repository's code. Deliberate exceptions use `#[expect(lint, reason = "...")]`, never workspace-wide allowances, and never without the maintainer's explicit approval for that specific case — the approved font palette in `arch/x86_64/src/terminal_fonts.rs` is such an exception and is not a precedent.

### 486 baseline (x86_32)

The x86_32 port runs on 486-class CPUs and must stay that way — the invariant, not the bug, is the default:

- **The pinned feature strings are the floor.** `arch/x86_32/rosx-i486.json` and `rosx-i486-user.json` compile against `-mmx,-sse,-sse2,-cmov,-fxsr,-cx8`; never widen them. `-cx8` pins `cmpxchg8b` out to keep the 486SX door open. Note: `RUSTFLAGS` in the environment *replaces* the `[target.rosx-*-user]` rustflags in `.cargo/config.toml` (which carry the `linker.ld` script) — never build user ELFs with an exported `RUSTFLAGS`.
- **No floating point on any 32-bit target** — kernel, usrlib, or apps. `swap_context` saves GPRs only, so any FP state would silently corrupt across task switches, and a 486SX has no FPU at all.
- **No `cpuid`-probed features** — `cpuid` itself is 586-era; the port runs flat protected mode with 386-era mechanism only (no paging, no CR4 bits).
- **Keep atomics ≤ 32 bits** on 32-bit paths; an `AtomicU64` there needs `cmpxchg8b`.
- **The machine guards:** `cargo xtask check-isa x86_32` audits the kernel and every user ELF against a 486 mnemonic allow-list, `cargo xtask check-isa x86_32 --stages` audits the pinned GRUB Legacy stage binaries against a post-486 denylist (CI runs both in `build-x86_32`), and every x86_32 QEMU session boots `-cpu 486`. A regression must fail one of these, never a board.
- **The two byte-gated allow-list exceptions:** `tzcnt` passes only in its legacy `f3`-prefixed form (executes as 386 `BSF`), and `ud2` passes only with the exact reserved `0f 0b` encoding — that opcode raises #UD identically on 486 silicon (Intel chose it for exactly that reason), and the compiler emits it only as a trap after a no-return panic path. Any other bytes decoding as `tzcnt`/`ud2` still fail the audit.
- Provenance: `plans/x86_32-i486-plan.md` and the evidence log `plans/x86_32-i486-impl.md`.

### Code Style

**Memory Safety & Ownership:**
- **CRITICAL:** Use Rust's safe memory management and ownership control as much as possible
- **`unsafe` usage:** Only use `unsafe` when absolutely unavoidable (e.g., raw hardware access, inline assembly)
- When `unsafe` is required:
  - Document WHY it's necessary
  - Document what invariants must be maintained
  - Keep `unsafe` blocks as small as possible
  - Provide safe wrappers around unsafe operations

**Hardware Abstraction:**
- **All hardware-specific routines MUST be abstracted from the kernel**
- Use Hardware Abstraction Layer (HAL) pattern - see the `Cpu` trait as the reference example
- Kernel code in `kernel/` should be completely platform-agnostic
- Platform-specific implementations go in `arch/[platform]/`
- Key HAL traits: `Cpu` (`kernel/src/platform/cpu.rs`), `ElfArch` (`kernel/src/platform/elf.rs`), `PortIo` (`kernel/src/platform/portio.rs`)

**No Test-Only Production Surface:**
- Every function, method, or constructor in production code must have at least one non-test caller; `pub(crate)` visibility is not an excuse
- Test scaffolding (inspectors, fixtures, wake/task helpers) lives inside the `#[cfg(test)]` module of the unit under test, never in production code paths
- If a test needs access beyond the production interface, reshape the test or give the function a real caller - never widen the production surface to serve a test
- Symmetric rule: production API without any caller (test or otherwise) is dead weight - wire a real user or delete it

**Pluggable Architecture:**
- **Scheduling is pluggable (implemented):** the strategy is a `SchedulingAlgorithm` (MLFQ + FIFO) chosen at boot via `scheduler_factory` in `KConfig` (`kernel/src/kconfig.rs`).
- **Memory is NOT pluggable (yet):** a single static `global_allocator` (`MemoryManager` → `FreeListAllocator`) is active; a `BitmapChunkAllocator` exists but is not wired in. Making the allocator selectable is a stated **low-priority goal**.
- Prefer trait-based design so subsystems can be swapped at boot time, not compile time.

**Architecture Principles (who owns what):**
- **Domain knowledge is owned by the allocating subsystem.** A kernel resource's identity, layout, and validity rules live with the subsystem that allocates and validates it; OS policy must not live in infrastructure crates.
- **The kernel is the single authority on invariants.** Validation happens once, kernel-side; user space carries opaque tokens and neither re-checks nor duplicates kernel invariants.
- **The shared treaty holds the contract, not the mechanism.** `system` contains numbers and layouts only: a type that describes *how the kernel works* is in the wrong crate; a type that describes *what the boundary contract says* belongs in `system`.
- **Translation lives at the boundaries.** The kernel edge encodes domain→wire and the user edge decodes wire→user-view; the shared crate cannot own either translation — the dependency arrows forbid it.
- **User-visible APIs expose no kernel internals.** Debug aids are kernel-side and never part of the ABI.

These principles are the regression guard: the very inversion this rework removed survived many runs because no document stated who owns handle semantics, while the HAL rule — written down — has been obeyed throughout.

**Kernel Subsystems (current invariants):**
- `future/` + `ipc/` — **notification-driven** futures (time, task-completion, depth-one waitset aggregates) and a mailbox IPC manager (bind/connect/send/receive); kernel-originated event delivery (`deliver_to_service`) wakes accepts on bound services; a waitset completes when any attached leaf future does, woken by a child-to-waitset cascade inside `FutureRegistry::notify`; no polling on the preemption cycle. The task-completion future carries the terminated task's exit status — see the exit-status channel.
- Interrupt discipline — kernel code runs with IF=0 except idle `hlt` and the user-return boundaries (`iretd`/`SFMASK`); the scheduler, future registry, and mailbox manager mutations are `IrqGuard` sections (`kernel/src/irq_guard.rs`) whose re-entry tripwire prints `[IRQGUARD]` and must stay silent. Forced context switches have a single site: wakers raise `need_resched` and `Kernel::maybe_resched()`, consulted only at the timer-IRQ tail and the syscall-exit wrapper tail, is the sole decision point — intent survives non-preemptible windows uncleared (`plans/stack/deferred-reschedule-impl.md`).
- `ipc/` data family — messages signal; data rides the connection out-of-band via write/read/release tokens; the kernel never interprets message payloads.
- Input architecture — policy lives in the user-space `INPUTD` daemon (`inputd`), device semantics in driver daemons (`ps2d`); the kernel delivers opaque event frames only.
- Disk architecture — ATA command semantics live in the user-space `ATAD` daemon (`atad`) over `ata-protocol`; the kernel contributes only the mediated port capability: the arch port declares the grantable ranges on `KConfig.port_pool` (x86: `x86_generic::port::ATA_PORT_POOL` = 0x1F0–0x1F7, 0x3F6) and a driver wins them at startup through the `PortAcquire` syscall — exclusive across tasks (`PortRangeBusy`), pool-bounded (`PortRangeNotInPool`), idempotent for the holder, surrendered by `PortRelease` or automatically at task termination; `atad` acquires both ranges before probing and exits with a failure status if refused. v1 polls; IRQ14 delivery is the v2 event path. On the same `ATA` binding `atad` also serves the generic **block family** of `block-protocol` (requests 0x20–0x2F, replies 0x30–0x3F, disjoint from the ata opcodes, decoded by band): `Probe` declares the device's honest geometry (atad declares 512-byte blocks = its sectors) plus a model/firmware description, `Read`/`Write` are block-addressed with a transfer count, `Flush` flushes the drive. The vendor face keeps sector truth for the `ata` CLI; `vfsd` speaks only the block family. **Block namespace:** the user-space `BLOCKD` daemon (`blockd`) is the single block-device provider: `Register <service>` binds a provider slot (`ata` → `ata0`), discovers its drives (probe device `0`, walk until `NotFound`, cap 8), and republishes them under globally unique stable names — `ata0d0`, `ata0d1`, … — where the `d`-form is reserved for discovered device leaves so service aliases can never collide with them. Client traffic carries one base-relative block number: blockd range-checks against `base + len`, forwards `base + block` to the slot's provider, and rewrites the device field on replies; `List`/`Probe` are served from the namespace table, only read/write/flush reach the provider. A re-`Register` refreshes the provider in place (same slot, only that slot's entries torn down and re-probed) and is not `Busy`; entries carry `kind` (`Disk`/`Partition`) and `state` (`Clean`/`Corrupt`). **Curation:** registering a provider reads each discovered drive's sector 0 through the provider and asks `rxpart` (`fs/partition`) for the exposure verdict: a valid MBR with at least one supported entry means the table wins — the whole drive does not enter the namespace and every supported slot becomes a slice device `ata{c}d{d}p{slot}` (zero-based true MBR entry index, holes skipped, `state=Clean`, `base=start_lba`, `len=sector_count`), so whole-disk/slice overlap is impossible by construction and no runtime overlap guard exists anywhere; no table, an empty table, or a table whose only entries are refused containers (0x05/0x0F/0x85, GPT-protective 0xEE) exposes the whole drive `Clean` with the refused types named in the description; a corrupt table exposes the whole drive as `Corrupt` — expose-on-corrupt keeps the rescue path open because the OS has no table writer. A provider fault during the sector-0 read fails `Register` exactly like a probe fault — nothing is exposed with a fabricated state. Registration logs one `DriveVerdict` line per drive (whole/partitions, refused container bytes verbatim) so every exposure decision is visible at `attach <service>` time. Vocabulary: a controller is `ata0`, a disk `ata0d1`, a slice `ata0d1p1`; `volume` names a mounted filesystem (rxfs or rxfat) and is never a partition. Staleness is the contract: re-`Register` re-reads sector 0 — Register *is* the rescan, there is no `Rescan` opcode. The vendor `ATA` face is the sharp knife: the `ata` CLI reads and writes whole-drive sectors past curation (diagnostic territory), so the blockd view and the `ata` view are complementary, not contradictory; the OS-side table authoring tool (`fdisk`-lite, a vendor-face client) stays its own plan with a busy/mounted handshake. blockd holds every provider connection for the whole boot (one per source, owned by the daemon task), which is why the VFS wire never carries service names. Norm, not enforcement: direct `"ATA"` dials stay available for diagnostics — the `ata` CLI keeps the vendor face — while `vfsd` and any future block consumer dial `"BLOCKD"` only. The `attach` CLI is blockd's thin client: bare lists the namespace grouped by source, `attach <service>` registers a controller; the VFS alias registry (opcodes 13/14) is retired and its numbers stay reserved.
- VFS architecture — the `VFS` daemon (`vfsd`) owns no block storage: it starts with an empty namespace and the mount table is the root. It is coupled to the `fs/core` `Engine` interface, not to a concrete filesystem: `VfsStore<D: BlockDevice>` holds mounted volumes as a `Vec<MountedVolume<D>>` table whose engine slot is a `MountedEngine<D>` — the enum `Rxfs(FileSystem<D>) | Fat(rxfat::FileSystem<RemoteWindow>)` implementing the `Engine` treaty itself, so dispatch, routing and Busy guards never learn which engine serves a slot (production `ActiveStore = VfsStore<RemoteWindow>`); every operation flows through the trait (method-call syntax, no inherent engine call). A cross-engine handle misuse (an rxfs handle against a fat slot or the reverse) is `BadHandle`. The kernel ROM never enters the VFS: `init` reads the ROM image itself through `RomRead`, and `vfsd` answers nothing before a mount. The wire carries paths: `valid_path` is always absolute — `/` itself (the root) or one or more `valid_name` segments joined by `/` bounded by the 60-byte frame — and routing looks a path's first segment up in the mount table — paths under a live named mount, bare or nested, go to that volume, the empty-named `/` mount swallows every otherwise-unclaimed path, everything else is `NotFound`. The root `/` itself always exists in the table: `stat` answers a directory of size 0, `list` answers the sorted named-mount names, and every file operation below an unmounted root stays `NotFound`. Listing is one operation (`LIST`, number 8, formerly `LIST_DIR`) following the same routing; the original argument-free `List` opcode (number 6) is retired and unused. A read-only `stat` query answers path → kind + size straight from `Engine::lookup` (zero open slots), and the shell launcher's file read is size-bounded so a mid-launch mutated image fails as "truncated" instead of executing. Ownership and open modes are enforced daemon-side (no engine knows either); `sync()` commits after every mutating request. `MKDIR`/`RMDIR` ride the wire onto `Engine::mkdir/rmdir`, and `NotDirectory`/`IsADirectory`/`DirectoryNotEmpty` are error bytes 10–12. **Device addressing:** `FORMAT`/`MOUNT` carry `blockd` device names — `vfsd` dials `"BLOCKD"` over `block-protocol::manager::client::Manager`, probes to fail early, rejects geometries the engine cannot adapt (`VfsError::Unsupported`, error byte 13) and owns no registry: the retired `ATTACH`/`LIST_DEVICES` opcodes (13/14) keep their numbers reserved and no VFS wire frame names a service. `format <device>`/`mount <device> <dir>` re-dial and re-probe fresh (unknown device → `NotFound` from blockd's `Probe`) and drive the volume through `RemoteWindow` (`apps/vfsd/src/blk.rs`), the `fs/core` `BlockDevice` adapter that maps one engine-unit block to `unit_bytes / declared_block_size` contiguous device blocks in one request — a 4 KiB window for rxfs, the volume's own sector size for FAT — so any mix of engines and device geometries mounts side by side, and answers `Unsupported`-sized geometries honestly. **Disk volumes:** a mount point is a directory that exists under a mounted `/` (the Linux rule): the caller `mkdir`s it through the live namespace first and `mount` validates it through the same routing (must exist and be a directory), and `rm`/`rmdir` refuse a live mount point as `NotPermitted`. The mount destination may also be `/` — the one mount point the VFS always has: `valid_mount_dir` accepts the root and the table records it as the empty-named entry. Routing stays first-segment: a named mount wins when the path's head equals its name, and the root mount swallows every other path — after a root mount unclaimed paths resolve on that volume and unknown prefixes answer from it rather than `NotFound`; mount-point validation for a later `MOUNT` flows through the same routing, so a named mount can be established at a directory that lives on the root-mounted volume. `/` itself stays `NotPermitted` to `rm`/`rmdir` under every configuration. The Busy guards: the mount table refuses a mounted device for `format` or a second mount point — blockd would happily hand out a second connection to the same disk, so this local guard is what keeps one disk from mounting twice — and mount names stay unique: a mounted device cannot be reformatted; unformatted/corrupt volume → `NotFormatted`. MOUNT auto-detects the engine before constructing it: the rxfs `FS_MAGIC` at superblock block 1 wins, else a boot sector that `rxfat`'s total parser accepts mounts FAT, neither → `NotFormatted` — the mount CLI and the wire never name a filesystem type, and `format` stays rxfs-only. Open files carry the table slot and the engine `MountHandle` of the volume they live on, and read/write/close dispatch through them. The daemon holds no mount name: "hdd" is pure caller convention.
- Kernel-hosted IPC services — `kstatsd` is the first: kernel code runs a service loop as a `FunctionTask` (launched by each arch at boot), calling `IpcManager` directly and blocking on futures via `Kernel::wait`. Two read-only operations share the `"KSTATSD"` binding: `MemorySnapshot` (op 0) answers in the 60-byte message; `ProcessTable` (op 1) rides the IPC data family — the reply message carries a data token (`system::ipc::encode_data_token`) and the table blob is a versioned fixed-stride buffer (`system::kstats::encode_process_table` / `decode_process_entry`), because a process list never fits 60 bytes. kstatsd snapshots the live task arena and per-group memory under one `IrqCell` section (the timer IRQ's resched path mutably borrows the task manager, so an unguarded walk would panic), then joins them: one row per loaded-image group, `pid` = the monotonic never-reused `GroupId`, one thread rolled up to the group's most-active state, memory = `MemoryManager::group_bytes` (that group's `Group` + `Stack` blocks). Metrics and identity stay domain-owned (`MemoryManager::stats`/`group_bytes`, `TaskManager::snapshot_processes`); the wire contract lives only in `system::kstats`; user-space clients (`mem`, `ps`) own rendering.
- **Boot plan** — the kernel schedules exactly `kstatsd` and `init`, and init's argv is a `system::boot_plan` script assembled by the arch `main.rs` with the fluent `BootPlan` builder. One argument per step: `service <program> <probe>` launches the program and retries `ipc_connect` until the service binds (~10 s budget), `run <program> [args…]` waits the tool's completion future, `exec <program> [args…]` is the hand-off after which init terminates. Every program token declares its source: `rom:<name>` reads a ROM entry, `/path` reads a VFS file through the typed client's size-bounded `read_whole` (a mid-boot mutated image fails as truncated, like the shell launcher), and a bare name is implicit-ROM — valid grammar, used by no shipped plan now that both ports boot the disk. A token that names no source panics the builder at add time and decodes `Malformed` in init. A failed `exec` no longer ends the plan: init falls through to the next step, so a disk-booting port ends `exec /shell` followed by `exec rom:shell` and still reaches a shell when the disk or its binary is missing. Plans stay inside the argv transport (argv0 plus ≤ 15 steps, ≤ 512 bytes; the builder asserts the bounds at encode). Failure policy is log-and-continue — `[INIT]` diagnostics always lead on to the shell. The arch ports use the protocol crates' `wire::SERVICE_NAME`/`SOURCE_NAME_*` constants as the probe strings for the five protocol-owned services, blockd's coming from `block_protocol::manager::wire`. The shared plan (identical on x86_64 and x86_32): `rom:atad`, `rom:blockd`, `run rom:attach ATA`, `rom:vfsd`, `rom:inputd`, `rom:ps2d` — the ROM bootstrap — then `run rom:mount ata0d0p0 /`, then `service /randomd RANDOM`, `service /echo BULKECHO`, `service /pong PING` sourced **from the mounted volume**, then the `/shell` → `rom:shell` hand-off pair; a missing disk leaves the services `not available` and lands on the ROM shell.
- `elf/` — loads standalone user-space ELF binaries into tasks (`new_elf_task_with_arguments`).
- `task/` + `task_manager/` — task lifecycle, context switch, preemption. Task stacks are sentinel-watermark-filled and verified at every switch-out (deep-watermark scan + saved-SP-vs-guard check sized to one swap frame): exhaustion panics with the task name — a permanent tripwire, like `[IRQGUARD]`. Every `LoadElf` request carries an argv blob whose argv0 is the program's identity, and the kernel names the task from it (`TaskName`, a 32-byte inline buffer, truncated on a char boundary) — no task is named `ELF` anymore.
- **Exit-status channel** — a task's exit outcome rides its paired completion future, never the `Task` (the scheduler frees the arena slot at termination). The future is self-contained: its completion is its own flag stamped together with the status, never polled from task state — so it stays valid and completed after the task slot is freed and recycled. The wire is an `i32` owned by `system::exit`: reason in bits 31..24 (`Clean`/`Panicked`; reserved reasons decode honestly), 24-bit code; `status_for_exit_request` remaps the reserved `EXIT_CODE_PANIC` request code to `Panicked(0)`, so user space can only ever carry `Clean` codes and out-of-range codes are refused (`ExitCodeOutOfRange`). The `Exit` syscall (number 4 — reused from the retired `ReadChar`, keeping the number space tidy; 9 stays retired) diverges through `Kernel::terminate_with_status`, which stamps and notifies the completion future before touching any child resource (state transition, port revoke, group reclaim), in the same registry borrow that wakes waiters — a waiter can never observe completion without its status, and a late waiter always consumes it. `elf_task_wrapper` maps main's return to `Clean` (high bits masked, no one to report to); `Spawn`/kernel `FunctionTask` keep `Clean(0)`; the usrlib panic handler prints and exits with the panic code instead of `loop {}`, so a dead daemon terminates and wakes its watchers. Delivery is the `WaitFuture` out-ptr (symmetric to the IPC payload) and consumes the future once — a second waiter gets `FutureAlreadyConsumed`, never a kernel panic; usrlib decodes the wire into the `ExitStatus` view (`wait_for_exit_status`), keeping the "parent" as whoever holds the completion-future handle. The shell reports anything but clean zero (`exiter exited with code 3`, `snake panicked`); `exiter` and the `exit_status` suite prove all three origins (main-return, `Exit`, panic redirect) on both arches.
- **Memory ownership = loaded-image groups** — user blocks are stamped `BlockOwner::Group`: `LoadElf` (and boot loads) starts a fresh group, `Spawn` siblings join the caller's group (they share that image's statics), and the kernel reclaims a group's memory when its last live member terminates. Task stacks carry `BlockOwner::Stack(group)` — accounted to the group but freed by the owning task's `Drop`, never by the bulk reclaim: reclaim runs while the last member's `Task` still lives, so stacks must stay out of its sweep (`deallocate_by_owner` matches `Group` only, never `Stack`).
- `rom/` — boot-registered immutable ROM image exposed via the opaque `RomRead` syscall; the kernel stays VFS-agnostic and the ROM is never a VFS volume — `init` is its only user-space reader, resolving every `rom:` plan token and the shell fallback from its own `read_image()`. The roster is per-arch (`ArchSpec.rom_apps`) and both ports pack `BOOTSTRAP_APPS` — the eight programs their plan names with `rom:` or needs before `/` exists, the shell included; the kernel's own copy of init is a separate `include_bytes!`. Its roster names stay underscore-free — a historical constraint from the era when the kernel keyboard dropped shift state, with the rename tracked in `docs/todo.md`.
- `fs/core` — the engine treaty of the `fs/` theme (`Engine` trait, device treaty `BlockDevice`, shared `FsError`/`IoError`, `Kind`, `DirItem`, `MetadataView`), zero deps beyond core/alloc. It is the in-process contract between `vfsd` and any filesystem: `Engine` names the twelve operations (`lookup/open/create/mkdir/rmdir/read/write/stat/close/remove/readdir/sync`) with associated `Handle` and `Metadata` types so each engine keeps its own handle representation and stat shape; `FsError::Unsupported` is the honest answer for a capability an engine lacks. `BlockDevice` is the one block-device contract every engine's constructor takes — rxfs and rxfat keep a `device` module that re-exports it so their local import paths stay put. Constructors stay engine-specific (`format`/`mount` for rxfs, `new` for rxfat and ramfs); `check()` and volume introspection stay on the concrete types. Both engines and `vfsd` depend on this crate and nothing else cross-cuts.
- `fs/rxfs` — standalone `no_std` filesystem library (zero third-party deps; links only `fs/core`): 4 KiB blocks, u32 addressing, static inode table with inline extents + spill chains, 255-byte byte-transparent names, Fletcher-32 on superblock and inodes. No journal — leak-safe commit ordering (allocation bitmaps hit disk before references, reclamation is deferred until `sync` commits the unlink), primary + backup superblock, and `check()` fsck-lite that repairs leaks, dangling entries, and double references. Proven by a crash-injection harness (every device-write prefix) and differential property tests; `FileSystem<D>` implements the `fs/core` `Engine` treaty and enters `vfsd` only as a mounted volume — the `Rxfs` arm of the auto-detecting `MountedEngine` (see VFS architecture); `MemoryDevice` is the in-memory `BlockDevice` stand-in the unit tests mount with.
- `fs/fat` — `rxfat`, the second mountable engine of the `fs/` theme (zero third-party deps; links only `fs/core`): a `no_std` FAT16/FAT32 read/write engine — total boot-sector parser (`0x55AA` signature, power-of-two sector sizes), FAT-table walk with per-handle chain cache, 8.3 slots displayed as lowercase display names, `create`/`write`/`mkdir`/`rmdir`/`remove` with cluster allocation, generation-checked handles. FAT12, valid-but-unimplemented sector sizes and foreign boot sectors answer `Unsupported`/`Corrupt` honestly; there is no formatter — `format` never writes FAT. Proven against real mtools images both directions (mformat seeds read through the engine, guest writes read back through mtools); `FileSystem<D>` implements the `Engine` treaty and mounts through `vfsd`'s auto-detecting `MountedEngine` table (see VFS architecture).
- `fs/ramfs` — second engine of the `fs/` theme: a `no_std` in-memory store (zero third-party deps; links only `fs/core`) implementing the full `Engine` treaty as rxfs does, over a `BTreeMap` keyed by full path, so `vfsd`'s generic `VfsStore` could run it unchanged. One path model (`path::parse`: absolute, no empty segments, 255-byte names, 4096-byte paths, depth ≤ 512) feeds every operation; parent chains resolve once — missing parent `NotFound`, file-as-parent `NotADirectory`. Entries carry their `Kind`: `lookup`/`readdir` tell files from directories, `open` and `remove` refuse directories as `IsADirectory`, `rmdir` refuses the non-empty as `DirectoryNotEmpty` and the root as `InvalidPath`. Generation-checked handles reject stale and deleted-file use as `BadHandle`; zero-fill past EOF; empty write is an `Ok(0)` no-op. It carries no daemon policy (no owner/mode, ROM, or wire-name rules — those stay at the daemon edge); `vfsd`'s `MountedEngine` still names only rxfs and rxfat as mountable arms.

**Documentation & Comments:**
- **CRITICAL: No comments, ever** — the code is the source of truth; make it self-explanatory
- Self-explain through:
  - Clear function names
  - Descriptive variable names
  - Well-structured logic
  - Type signatures that document intent
- Exception: Assembly code must have comments (register usage, calling conventions)

**General Style:**
- Follow standard Rust conventions
- Keep functions focused and modular
- Prefer type safety over raw pointers/integers where possible
- Use descriptive names for types, functions, and variables

---

### Build Issues?
- Toolchain: `rust-toolchain.toml` pins **nightly** with the `rust-src` component (`rustup component add rust-src`).
- Custom targets: `arch/x86_64/rosx.json` (+ `rosx-user.json`), `arch/x86_32/rosx-i486.json` (+ `rosx-i486-user.json`).
- No bootimage — the disk image comes from `arch/x86-runner` (the `bootloader` crate). To build it standalone: `cargo run --manifest-path arch/x86-runner/Cargo.toml -- <kernel-binary> x86_64 --no-run`.

### GitHub CLI?
- `gh pr edit` on `rghisi/rosx` fails here with a Projects (classic) deprecation GraphQL error (`repository.pullRequest.projectCards`) even when touching no project field. Update PRs through the REST API instead: `gh api -X PATCH repos/rghisi/rosx/pulls/<n> -f title="..." -F body=@<file>`.

---

## Notes for AI Assistants

### ⚠️ MOST IMPORTANT - Work Incrementally ⚠️

**MANDATORY: Work in very small steps - NEVER write code without explanation and approval**

This is the MOST CRITICAL guideline for working on RosX:

1. **Propose changes BEFORE writing code**
   - Explain what you plan to do
   - Explain WHY you're doing it
   - Describe the approach you'll take
   - Ask for feedback/approval

2. **Work in tiny increments**
   - One small change at a time
   - One function, one file, one concept
   - Make it work, verify, then move to next step
   - Before moving to the next step, ask for confirmation, create a commit message and confirm
   - The commit message should not contain any prefix

3. **Never batch multiple changes**
   - Don't implement several features at once
   - Don't modify multiple files without discussing each
   - Don't assume the next step - always ask

4. **Continuous communication**
   - Explain your reasoning
   - Discuss trade-offs
   - Share alternative approaches
   - Wait for user feedback before proceeding

**This is a learning project - the journey and understanding are as important as the destination.**

---

### Other Critical Guidelines

- **CRITICAL: Minimize `unsafe` usage** - Use Rust's safe abstractions whenever possible; `unsafe` only when truly unavoidable
- **CRITICAL: Hardware abstraction required** - All platform-specific code must go through HAL traits (like `Cpu`), never directly in kernel code
- **CRITICAL: kernel/ is `no_std`** - std is forbidden; dependencies are limited to the workspace crate `system`. No other third-party crates.
- **CRITICAL: No comments, ever** - Write self-explanatory code with clear names instead
- **CRITICAL: Write unit tests** - All kernel modules should have comprehensive unit tests (see `kernel/src/scheduler/mlfq_strategy.rs` as a reference)
- **CRITICAL: No test-only functions in production code** - production functions need non-test callers; test helpers belong in `#[cfg(test)]` modules, and production APIs need real users
- Multi-platform support is a goal - keep kernel code portable and platform-agnostic
- Build and test after significant changes (`cargo test -p kernel`)
