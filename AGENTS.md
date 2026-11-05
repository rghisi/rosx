# RosX - Agent Context Document

> **Last Updated:** 2026-11-04
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
│    elf/ ipc/ (mailbox, data) future/ syscall/ kstatsd task*  platform/ (cpu, elf, interrupt, output, portio HAL traits + per-task port grants)  kconfig  rom/
├── system/      # ABI treaty: syscall numbers, wire layout, ROM image layout, error numbers
├── usrlib/      # User-space libc: allocator (binned slabs, global default), syscall, out, arch/{x86_64,x86_32}
├── protocol/                # Service RPC treaties, one sub-crate each (workspace glob protocol/*)
│    vfs/         # package vfs-protocol: wire codec + typed client
│    input/       # package input-protocol: wire codec
│    ata/         # package ata-protocol: wire codec + typed client
│    block/       # package block-protocol: generic block-device family (reserved opcode band) — wire codec + dial-by-name client
├── fs/
│    core/        # engine treaty (trait Engine + FsError/Kind/DirItem/MetadataView), no deps, shared by rxfs, ramfs and vfsd
│    rxfs/        # no_std filesystem library (vfsd's engine): superblock/bitmaps/inodes/extents/dirs, leak-safe sync, check()
│    ramfs/       # no_std in-memory filesystem engine, rxfs-shaped (lookup/open/create/read/write/close/remove/readdir/stat) over a flat byte-name store
├── arch/
│    x86_64/        # entry, cpu, interrupts, framebuffer, terminal_fonts, *.S (global_asm)
│    x86_32/        # entry, cpu, interrupts, boot.S (Multiboot v1)
│    x86-generic/   # shared x86-family mechanism: port executor, VGA text buffer, debug console, ANSI parser, Multiboot v1 memory decode
│    x86-runner/    # workspace member: kernel → boot medium + QEMU launcher for both ports (NOT bootimage)
│    x86-86box/     # data-and-scripts (no crate): era-VM template, GRUB Legacy media recipes, provenance manifest, runbook
└── apps/
     [workspace members]  shell, dummy, pi, randomd, echo, bulk, pong, ping, mem, vfsd, vfs, snake, tetris, conway, threads, waitset, stress, inputd, ps2d, atad, ata, bench, format, mount, attach, rm, cp, mkdir, rmdir
```

---

## Build & Run

Canonical recipes: `.github/workflows/ci.yml` (build + test) and `build-artifacts.yml` (disk images).

### x86_64
```bash
cargo xtask build              # ELF apps + ROM image + kernel + disk image
cargo xtask apps               # ELF apps only
cargo xtask rom                # ROM image from built apps
cargo xtask test               # all host unit tests
cargo build -p rosx            # kernel only (from arch/x86_64 directory or with target)
cargo run -p rosx              # run.sh -> arch/x86-runner -> BiosBoot disk image -> QEMU
```
- `cargo run` uses the **custom runner** (`.cargo/config.toml` → `./run.sh`), which invokes `arch/x86-runner` to make a BIOS disk image with the `bootloader` crate's `BiosBoot`, then starts `qemu-system-x86_64`. **Not bootimage.**
- Both `cargo run` and `cargo xtask run` attach a persistent host-side 50 MiB IDE disk (`$HOME/tmp/rosx-hdd.img`, created if missing, never truncated, survives VM restarts): `ata list` → `attach <dev-id> ATA <drive>` → `format <dev-id>` → `mkdir <dir>` → `mount <dev-id> <dir>` → `ls <dir>` (the mount point is a real root-volume directory that must exist before mounting; the attach registry is in-memory in `vfsd`, so every boot re-attaches). Footgun: `format 0` on x86_64 rewrites the boot image file (harmless — the next build regenerates it). The headless integration harness keeps its own tempdir disks (`QemuSession::spawn_with_hdd_disk` for the persisting flow).
- Target spec: `arch/x86_64/rosx.json` — bare-metal `no_std`, `build-std` = core/alloc/compiler_builtins.
- User-space ELF apps (pi, randomd, snake, tetris, conway) are workspace members; build with `cargo xtask apps` or `cargo xtask build`.
- The shell is a user ELF: each arch port embeds `target/rosx-user/release/shell` with `include_bytes!` and boots it via `new_elf_task`. It keeps five built-ins (`ls clear rose sleep random`) and resolves every other prompt name through VFS, so ROM/RAM files execute by name with arguments (`rose> snake fast`).
- `cargo build --workspace` is intentionally unsupported: bare-metal bins and PIE apps cannot compile for the host.

### x86_32
```bash
cargo xtask build x86_32       # ELF apps + ROM image + kernel (Multiboot v1, boot.S)
cargo xtask run x86_32         # boot QEMU (-kernel, qemu-system-i386) to the shell prompt
cargo xtask image x86_32       # GRUB bootable ISO (needs grub-mkrescue, xorriso, mtools)
cargo build -p rosx-x86_32     # kernel only (from arch/x86_32/ — its .cargo/config.toml supplies the rosx-i486 custom target)
```
- The kernel embeds the shell ELF and the ROM image that `cargo xtask build x86_32` produces under `target/rosx-i486-user/release/`; `arch/x86_32/build.rs` asserts both are present before compiling.
- The boot roster matches x86_64 (randomd, echo, bulk, pong, vfsd, inputd, ps2d, atad with ATA port grants, then the shell), so programs run by name from the prompt and the disk stack works there.
- Integration tests run on x86_32 with `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1`; the harness boots the kernel ELF directly and synthesizes the two-drive IDE topology for the ata suites. `ipc_vfs` and `ata_env_spike` stay x86_64-only; `kstats_mem` runs unguarded on both arches (the wake-path race is fixed — `plans/x86_32-wake-race.md`).
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
cargo test -p ramfs            # in-memory filesystem engine (rxfs-shaped flat store)
```

### Integration tests (run on host against QEMU)
```bash
cargo test -p tests-integration -- --test-threads=1                      # x86_64
ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1      # x86_32
```

### xtask commands
```bash
cargo xtask build [arch] [--debug]  # ELF apps + ROM + kernel + boot artifact (arch: x86_64 default, or x86_32)
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
- `future/` + `ipc/` — **notification-driven** futures (time, task-completion, depth-one waitset aggregates) and a mailbox IPC manager (bind/connect/send/receive); kernel-originated event delivery (`deliver_to_service`) wakes accepts on bound services; a waitset completes when any attached leaf future does, woken by a child-to-waitset cascade inside `FutureRegistry::notify`; no polling on the preemption cycle.
- Interrupt discipline — kernel code runs with IF=0 except idle `hlt` and the user-return boundaries (`iretd`/`SFMASK`); the scheduler, future registry, and mailbox manager mutations are `IrqGuard` sections (`kernel/src/irq_guard.rs`) whose re-entry tripwire prints `[IRQGUARD]` and must stay silent. Forced context switches have a single site: wakers raise `need_resched` and `Kernel::maybe_resched()`, consulted only at the timer-IRQ tail and the syscall-exit wrapper tail, is the sole decision point — intent survives non-preemptible windows uncleared (`plans/stack/deferred-reschedule-impl.md`).
- `ipc/` data family — messages signal; data rides the connection out-of-band via write/read/release tokens; the kernel never interprets message payloads.
- Input architecture — policy lives in the user-space `INPUTD` daemon (`inputd`), device semantics in driver daemons (`ps2d`); the kernel delivers opaque event frames only.
- Disk architecture — ATA command semantics live in the user-space `ATAD` daemon (`atad`) over `ata-protocol`; the kernel contributes only the mediated port capability (`portio` grants). v1 polls; IRQ14 delivery is the v2 event path. On the same `ATA` binding `atad` also serves the generic **block family** of `block-protocol` (requests 0x20–0x2F, replies 0x30–0x3F, disjoint from the ata opcodes, decoded by band): `Probe` declares the device's honest geometry (atad declares 512-byte blocks = its sectors) plus a model/firmware description, `Read`/`Write` are block-addressed with a transfer count, `Flush` flushes the drive. The vendor face keeps sector truth for the `ata` CLI; `vfsd` speaks only the block family.
- VFS architecture — the `VFS` daemon (`vfsd`) is coupled to the `fs/core` `Engine` interface, not to a concrete filesystem: `VfsStore<E: Engine, D: BlockDevice>` holds the root engine as `E` and mounted volumes as a `Vec<MountedVolume<D>>` table (production `ActiveStore = VfsStore<ActiveVolume, RemoteBlock>`), so every operation flows through the trait (method-call syntax, no inherent engine call). `VfsStore::new()` fixes the root engine to rxfs (`ActiveVolume = rxfs::FileSystem<MemoryDevice>`) on a fixed 8 MiB volume (2048 blocks, 512 inodes, formatted fresh each boot — the ROM volume), and swapping to ramfs is a one-alias change. The kernel ROM image is ingested as regular files and stays write/delete-denied via the daemon's name-set. The wire carries paths: `valid_path` is one or more `valid_name` segments joined by `/` bounded by the 60-byte frame, and routing looks a path's first segment up in the mount table — bare and nested paths under a live mount point go to that volume, unknown prefixes are `NotFound`, everything else maps to `/{path}` on the root engine. A read-only `stat` query answers path → kind + size straight from `Engine::lookup` (zero open slots), and the shell launcher's file read is size-bounded so a mid-launch mutated image fails as "truncated" instead of executing. Ownership and open modes are enforced daemon-side (no engine knows either); `sync()` commits after every mutating request. `MKDIR`/`RMDIR` ride the wire onto `Engine::mkdir/rmdir`, and `NotDirectory`/`IsADirectory`/`DirectoryNotEmpty` are error bytes 10–12. **Device attach:** `vfsd` owns the device registry in memory — `attach <dev-id> <service> <device>` dials the provider by name over `block-protocol::client::Block`, probes it to fail early, rejects geometries rxfs cannot adapt (`VfsError::Unsupported`, error byte 13) and stores alias → (service, device, geometry) one-shot until reboot; `attach list` renders the table over the `Listed` data-object plumbing. `format <dev-id>`/`mount <dev-id> <dir>` address the registry entry (unknown alias → `NotFound` before any dial), re-dial and re-probe fresh, and drive the volume through `RemoteBlock` (`apps/vfsd/src/blk.rs`), the rxfs `BlockDevice` adapter that maps one 4 KiB fs block to `BLOCK_BYTES / declared_block_size` contiguous device blocks in one request and answers `Unsupported`-sized geometries honestly. **Disk volumes:** a mount point is a real root-volume directory the caller `mkdir`s first; `mount` validates it through `Engine::lookup` (must exist and be a directory) and overlays it while mounted — bare-name ops route to the volume root, and `rm`/`rmdir` refuse a live mount point as `NotPermitted`. The Busy guards: attach refuses a duplicate alias or a duplicate (service, device) identity, so one disk never attaches twice; the mount table refuses a mounted device for `format` or a second mount point, and mount names stay unique — a mounted device cannot be reformatted; unformatted/corrupt volume → `NotFormatted`. Open files carry an `OpenLocation` — a root `E::Handle` or a table slot plus its rxfs handle — and read/write/close dispatch through it. The daemon holds no mount name: "hdd" is pure caller convention.
- Kernel-hosted IPC services — `kstatsd` is the first: kernel code runs a service loop as a `FunctionTask` (launched by each arch at boot), calling `IpcManager` directly and blocking on futures via `Kernel::wait`. Metrics stay domain-owned (`MemoryManager::stats`); the wire contract lives only in `system::kstats`; user-space clients (`mem`) own rendering.
- `elf/` — loads standalone user-space ELF binaries into tasks (`new_elf_task`).
- `task/` + `task_manager/` — task lifecycle, context switch, preemption. Task stacks are sentinel-watermark-filled and verified at every switch-out (deep-watermark scan + saved-SP-vs-guard check sized to one swap frame): exhaustion panics with the task name — a permanent tripwire, like `[IRQGUARD]`. Boot ELF tasks carry real names; shell-launched (`LoadElf`) tasks are still named `ELF`.
- **Memory ownership = loaded-image groups** — user blocks are stamped `BlockOwner::Group`: `LoadElf` (and boot loads) starts a fresh group, `Spawn` siblings join the caller's group (they share that image's statics), and the kernel reclaims a group's memory when its last live member terminates.
- `rom/` — boot-registered immutable ROM image exposed via the opaque `RomRead` syscall; the kernel stays VFS-agnostic, `vfsd` owns mount semantics. The ROM carries the app ELFs (not the shell); its roster names stay underscore-free — a historical constraint from the era when the kernel keyboard dropped shift state, with the rename tracked in `docs/todo.md`.
- `fs/core` — the engine treaty of the `fs/` theme (`Engine` trait + shared `FsError`/`IoError`, `Kind`, `DirItem`, `MetadataView`), zero deps beyond core/alloc. It is the in-process contract between `vfsd` and any filesystem: `Engine` names the twelve operations (`lookup/open/create/mkdir/rmdir/read/write/stat/close/remove/readdir/sync`) with associated `Handle` and `Metadata` types so each engine keeps its own handle representation and stat shape; `FsError::Unsupported` is the honest answer for a capability an engine lacks. Constructors stay engine-specific (`format`/`mount` for rxfs, `new` for ramfs); `check()` and volume introspection stay on the concrete types. Both engines and `vfsd` depend on this crate and nothing else cross-cuts.
- `fs/rxfs` — standalone `no_std` filesystem library (zero third-party deps; links only `fs/core`): 4 KiB blocks, u32 addressing, static inode table with inline extents + spill chains, 255-byte byte-transparent names, Fletcher-32 on superblock and inodes. No journal — leak-safe commit ordering (allocation bitmaps hit disk before references, reclamation is deferred until `sync` commits the unlink), primary + backup superblock, and `check()` fsck-lite that repairs leaks, dangling entries, and double references. Proven by a crash-injection harness (every device-write prefix) and differential property tests; `FileSystem<D>` implements the `fs/core` `Engine` treaty and is wired in as `vfsd`'s storage engine over `MemoryDevice` (see VFS architecture).
- `fs/ramfs` — second engine of the `fs/` theme: a `no_std` in-memory store (zero third-party deps; links only `fs/core`) implementing the same `Engine` treaty as rxfs over a flat byte-name map, so `vfsd`'s generic `VfsStore` could run it unchanged. Generation-checked handles reject stale and deleted-file use as `BadHandle`; zero-fill past EOF; empty write is an `Ok(0)` no-op; root is the only directory, so `mkdir`/`rmdir` answer `FsError::Unsupported`. It carries no daemon policy (no owner/mode, ROM, or wire-name rules — those stay at the daemon edge); `vfsd` still selects rxfs as its active `Engine`.

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
