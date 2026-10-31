# x86-Generic Arch Crate — Refactoring Plan

> **Status:** Plan (no code written). Written 2026-10-31.
> **Branch base:** `origin/main` (`7e1e6ae`, waitset #76; includes #86). The
> port-I/O mechanism and the ATA integration suite live on the unmerged
> `ata-pio-driver-plan` branch: milestones M1, M3, M4 apply to `main` today
> (the `ansi_parser`, `debug_console`, `vga_buffer` duplication exists there);
> **M2 lands only once `portio` is merged into `main`** (or its implementing
> branch is rebased onto the merged main). The §2 portio facts were measured
> on the `ata-pio-driver-plan` tree.
> **Feature:** a new workspace crate `arch/x86-generic/` holding code that is
> x86-family-specific but not bit-width-specific, shared by `arch/x86_64` and
> `arch/x86_32`. First residents: the port executor (the `PortIo` HAL
> implementation), then the duplicated display/output modules.
> **Method:** every repo fact below was verified against this working tree
> (paths cited; duplication measured by `diff`/`wc` on 2026-10-31). Commands
> this plan cannot prove green are marked **`unverified`**.

---

## 1. Goal & Non-Goals

### Goal
- Give the two x86 ports one home for shared mechanism, so fixes land once
  instead of twice, while the platform-agnostic rule for `kernel/` is untouched.
- Scope rule (the crate's whole reason to exist, written into AGENTS.md):
  **x86-family-specific but not bit-width-specific** code lives in
  `arch/x86-generic/`; anything else stays in its port.

### Non-Goals
- No new kernel mechanism. The `PortIo` trait and the per-task grant table
  stay exactly where they are (`kernel/src/portio.rs`), following the `Cpu`
  precedent: trait defined in the kernel, implemented outside it. The kernel
  must **never** depend on `x86-generic` — the arrow points one way only.
- No unification of the two interrupt schemes (PIC dispatch-core vs direct),
  of `cpu.rs`, or of `elf_arch.rs`. Each is explicitly deferred below.
- No behavior changes anywhere: this is a pure relocation refactor, provable
  by byte-identical duplication or by the existing integration tests.

## 2. Verified baseline facts (2026-10-31, paths)

1. **`ansi_parser.rs` is byte-identical duplication.** 197 lines in both
   `arch/x86_64/src/` and `arch/x86_32/src/`; `diff -q` reports no difference.
2. **`portio.rs` is semantic duplication.** `arch/x86_64/src/portio.rs` (40
   lines) implements `kernel::portio::PortIo` over the `x86_64` crate's
   `Port<T>`; `arch/x86_32/src/portio.rs` (82 lines) implements the same trait
   over hand-written `asm!("in al, dx", ...)` / `asm!("out dx, al", ...)`
   helpers with `options(nomem, nostack, preserves_flags)`. The four trait
   methods (read/write with `PortWidth`, read_words/write_words loops) are
   behaviorally identical.
3. **The x86_32 asm form is the portable one, and is already proven on
   x86_64 in-tree.** `arch/x86_64/src/debug_console.rs` emits the same
   `out dx, al` idiom directly for port 0xE9. `in`/`out` with `dx`-port
   addressing encodes identically in 32- and 64-bit mode. Conversely the
   **`x86_64` crate cannot be a dependency of the shared crate**: it does not
   compile for the `rosx-i686` target, while both ports must build the shared
   crate (`arch/x86_32/Cargo.toml` today depends only on `kernel`, `system`,
   `input-protocol`, `spin`, `volatile`, `lazy_static` — no `x86_64`, no
   `pic8259`).
4. **`debug_console.rs` is cosmetic duplication.** Same ISA debug console on
   port 0xE9; x86_64 factors the asm into a `write_byte` helper with a
   `DEBUG_PORT` const, x86_32 inlines the same `asm!` in the write loop
   (25 vs 18 lines).
5. **`vga_buffer.rs` differs only in physical-memory offset handling.** 254
   vs 245 lines; the functional delta is the `VGA_PHYS_OFFSET: AtomicU64` +
   `pub fn init(physical_memory_offset)` + the offset load in the buffer
   accessor on x86_64, versus a bare `0xb8000 as *mut Buffer` on x86_32
   (identity-mapped, no offset). Two comment-only lines also differ.
6. **Wiring sites.** `kernel::portio::PortIo` (trait, `PortWidth`, grant table,
   `GRANT_CAPACITY = 8`) is consumed via `KConfig { port_io: &PORT_IO, ... }`
   at `arch/x86_64/src/main.rs:44-49` and `arch/x86_32/src/main.rs:17-22`.
   These two `KCONFIG` literals are the only construction sites.
7. **Workspace shape.** `arch/x86_64` and `arch/x86_32` are `members` of the
   workspace `Cargo.toml`; both already depend on `kernel` and `system`, so an
   `x86_64 -> x86-generic -> kernel` edge adds no new *kind* of arrow. Custom
   targets: `arch/x86_64/rosx.json`, `arch/x86_32/rosx-i686.json` (path
   dependencies compile like any other crate; no target-file change needed).
8. **Real-hardware-oracle tests exist for the port path.**
   `tests-integration/tests/ata_identify.rs`-style boot reporting plus the
   read/write integration tests (commits "Read ATA sectors through atad…"
   and "Write and flush ATA sectors against a scratch slave drive under
   QEMU") exercise `read`/`write`/`read_words`/`write_words` end-to-end in
   QEMU — the strongest regression gate for the portio relocation.
9. **Repo rules this plan must honor** (AGENTS.md): no comments ever (the two
   stray comments found in x86_64's `vga_buffer.rs` are **not** carried into
   the shared crate); **no test-only production surface** (every moved
   function already has real callers in both ports — none may be added "for
   tests"); `kernel/` dependency rule is untouched because kernel does not
   gain a dependency; host `#[cfg(test)]` tests required where logic moves
   (`ansi_parser` carries its own); commit messages carry no prefix.

## 3. Design decisions

### 3.1 Placement and naming
New crate **`arch/x86-generic/`**, package name `x86-generic` (lib target
`x86_generic`). The `arch/` prefix keeps the "platform-specific code lives
under arch/" rule literally true, and the name follows the existing
`arch/x86_64-runner` precedent of naming by what it is, not by target triple.

### 3.2 Dependency arrows
```
arch/x86_64 --> arch/x86-generic --> kernel --> system
arch/x86_32 --> arch/x86-generic --> kernel --> system
```
- The single reason `x86-generic` depends on `kernel` is to implement
  `kernel::portio::PortIo` (like every arch already implements `Cpu`).
- Rejected alternative: keeping `x86-generic` a leaf crate with inherent
  methods and leaving a per-port `impl PortIo` shim. It preserves a
  zero-dependency crate at the price of re-duplicating the 4-method dispatch
  glue in both ports — the exact pattern this plan removes. The HAL rule is
  about keeping `kernel/` free of platform knowledge, which this respects.
- Rejected alternative: moving the `PortIo` trait *into* `x86-generic`. That
  would force `kernel` to depend on it (trait must be nameable by `KConfig`),
  violating "kernel/ dependencies are limited to `system` + `lazy_static`"
  and inverting the HAL pattern (`Cpu`, `ElfArch`, `InterruptController` all
  define the trait in the kernel).
- `lazy_static` and `spin` stay available to the crate but the first batch of
  modules needs neither.

### 3.3 The shared port executor uses the asm form, everywhere
`arch/x86_64`'s `x86_64`-crate-based implementation is replaced by the
x86_32 asm form (§2.3). One implementation, compiled for both custom targets.
`options(nomem, nostack, preserves_flags)` and the `unsafe`-wrapper style are
carried over verbatim; the asm comment exception does not apply (mnemonics
match operand names — the code is self-explaining).

### 3.4 vga_buffer offset injection
The shared buffer keeps x86_64's design: a `VGA_PHYS_OFFSET` static plus
`pub fn init(offset)` called by the arch that needs it. x86_32 either never
calls `init` (default 0 reproduces identity mapping exactly, since
`0 + 0xb8000 == 0xb8000`) or calls it with 0 — decide at implementation by
whichever keeps both `main.rs` files more honest; behavior is identical
either way, so this is not a correctness fork.

### 3.5 Host-testability
`x86-generic` follows the ps2d app pattern: `#![cfg_attr(not(test), no_std)]`
so the `ansi_parser` host tests keep running under `cargo xtask test`
(`cargo test --workspace`). The asm port-executor functions get **no host
tests** (executing `in`/`out` in user space would fault); they are covered by
the QEMU integration suite, exactly as `arch/x86_64/src/portio.rs` is today.
No new test-only API is introduced anywhere in the crate.

## 4. What moves, what stays (the measured list)

| Module | Verdict | Evidence (§2) |
|--------|---------|---------------|
| `ansi_parser.rs` | **moves, verbatim** | byte-identical |
| `portio.rs` (executor + `impl PortIo`) | **moves, asm form** | semantic dup; x86_32 form already proven on x86_64 via `debug_console.rs` |
| `debug_console.rs` | **moves**, keeping the `DEBUG_PORT` const + `write_byte` shape | cosmetic dup |
| `vga_buffer.rs` | **moves** with the offset static (§3.4) | 4 functional lines differ |
| `cpu.rs` | stays | x86_64 half is EFER/LSTAR/STAR/SFMask + GDT segments — no x86_32 twin |
| `elf_arch.rs` | stays | ELF32 vs ELF64 `ElfArch` impls — that is bit-width policy |
| `interrupts.rs` | stays | different IDT/entry mechanics per width |
| PIC controller (`interrupt_controller.rs`, pic8259) | **deferred** (see §6) | x86_32 predates the dispatch-core migration |
| PIT programming | **deferred** (with PIC) | lives inside x86_64 `interrupts.rs` |
| `apic.rs`, `framebuffer.rs` | stays | x86_64-only |
| `*.S` (context switch, boot, syscall) | stays | inherently per-port |

## 5. Milestones (each one commit; approve-and-go per step)

### M1 — Crate scaffold + `ansi_parser` (zero-risk proving flight)
- New `arch/x86-generic/Cargo.toml` (`#![cfg_attr(not(test), no_std)]` lib),
  `src/lib.rs`, `src/ansi_parser.rs` moved verbatim (both arch copies deleted;
  both `main.rs` switch to `x86_generic::ansi_parser`), workspace `members`
  entry. `default-members` stays unchanged (bare `cargo test` posture
  preserved; `cargo test --workspace` via `cargo xtask test` covers it).
- **Gate:** `cargo xtask test --skip-integration` · `cargo xtask build` ·
  from `arch/x86_32/`: `cargo build -p rosx-x86` (**`unverified`** baseline —
  orchestrator proves it green pre-change first).
- **Commit:** `Extract the duplicated ANSI parser into a new x86-generic arch crate`

### M2 — Port executor consolidation (the substantive step)
- `src/port.rs`: the six asm primitives (in/out × byte/word/dword) + a
  `X86PortIo` struct implementing `kernel::portio::PortIo` (block loops
  included), in the x86_32 form (§3.3).
- Both `main.rs`: `static PORT_IO` points at `x86_generic::port::X86PortIo`;
  `arch/x86_64/src/portio.rs` and `arch/x86_32/src/portio.rs` deleted; the
  `mod portio;` lines go. Grant policy calls (`kernel::portio::grant(...)` in
  x86_64 `main.rs`) are untouched — policy stays in the kernel.
- **Blast radius:** every existing port syscall path (kstatsd untouched,
  atad/ata are the real users). No kernel change; no treaty change.
- **Gate:** `cargo xtask test --skip-integration` · `cargo xtask build` ·
  `cargo build -p rosx-x86` · full `cargo xtask test` — the ATA QEMU
  integration tests are the behavioral oracle (§2.8).
- **Commit:** `Share the x86 port executor between both arch ports`

### M3 — Debug console
- `src/debug_console.rs` as §2.4's unified shape (`DEBUG_PORT = 0xE9`,
  `write_byte` helper, `fmt::Write` impl); both arch statics
  (`DEBUG_CONSOLE`) repoint; local copies deleted.
- **Gate:** same quartet; the debug console is the `-debugcon stdio` channel
  every integration test reads, so `cargo xtask test` is the direct oracle.
- **Commit:** `Share the ISA debug console between both arch ports`

### M4 — VGA text buffer
- `src/vga_buffer.rs` with the `init(offset)` design (§3.4), comments not
  carried over (AGENTS.md); x86_64 `main.rs` calls `init(phys_offset)` as
  today, x86_32 keeps identity behavior; local copies deleted.
- **Gate:** the quartet + visual/boot-banner check both arches
  (`cargo run`; x86_32 image via `arch/x86_32/build-image.sh` if the
  toolchain is present — else record as skipped, `unverified`).
- **Commit:** `Share the VGA text buffer between both arch ports`

### M5 — Documentation sweep (can join M1 and M4 commits where facts change)
- `AGENTS.md` structure block: add the `x86-generic/` line under `arch/`
  ("# shared x86-family mechanism: port executor, VGA text buffer, debug
  console, ANSI parser"). The `PortIo` HAL-trait line stays as-is (trait is
  still `kernel/src/portio.rs`).
- Verify no other AGENTS.md line was invalidated (`grep -n "arch/" AGENTS.md`).

## 6. Deferred, recorded not dropped
- **PIC 8259 into x86-generic** — the init/mask/eoi sequences are shared ISA
  mechanism and belong here eventually, but x86_32 still uses its pre-#77
  direct-PIC scheme; unification requires migrating x86_32 to the
  `kernel/src/interrupt.rs` dispatch core **first**, as its own plan.
- **PIT programming** — same dependency (lives in x86_64 `interrupts.rs`
  today; shared once both ports are dispatch-core users).
- `cpu.rs`, `elf_arch.rs` — genuinely bit-width-specific; no move candidate.
- If a future `arch/i586`-style port appears, it inherits the same crate.

## 7. Risks

| # | Risk | Mitigation |
|---|------|------------|
| 1 | Shared asm regresses x86_64 port behavior vs the `x86_64`-crate version | The two forms emit the same instructions; the ATA integration suite drives real in/out on both widths of transfer (scalar + block) and fails loudly |
| 2 | `x86_64` crate accidentally re-enters x86-generic (breaks i686 build) | The `cargo build -p rosx-x86` gate runs at every milestone and fails instantly |
| 3 | vga_buffer offset refactor silently breaks one port's console | x86_32 path is arithmetic-identical with offset 0 (§3.4); integration tests read console output on x86_64; M4 adds the x86_32 visual check |
| 4 | Workspace `cargo test --workspace` gains a no-std compile failure on host | `cfg_attr(not(test), no_std)` pattern already proven by ps2d/vfsd libs (M1 gate catches it immediately) |
| 5 | Crate accretes bit-width-specific code over time | Scope rule written into AGENTS.md at M5; reviewers check `arch/x86-generic/src` against "x86-family, not bit-width" |

## 8. Verification matrix

| Command (cwd) | Proves | Status |
|---|---|---|
| `cargo xtask test --skip-integration` (root) | host suites incl. moved ansi_parser tests | green today |
| `cargo xtask build` (root) | both… no: builds the x86_64 kernel + apps + ROM + image | green today |
| `cargo build -p rosx-x86` (`arch/x86_32/`) | shared crate compiles for rosx-i686 | **unverified** until orchestrator baseline |
| `cargo xtask test` (root) | QEMU suite incl. ATA port-I/O oracles | needs `qemu-system-x86_64` |
| `cargo run` (root) | interactive boot + console | manual companion |

## 9. Interfaces summary (the crate's whole public surface after M4)
- `x86_generic::port::{X86PortIo}` — implements `kernel::portio::PortIo`.
- `x86_generic::debug_console::QemuDebugConsole` — `fmt::Write` sink on 0xE9.
- `x86_generic::vga_buffer::{VgaWriter, init}` — text-mode output at
  `offset + 0xb8000` (exact type names as found during the move).
- `x86_generic::ansi_parser` — moved verbatim with its host tests.

No kernel, treaty, or protocol interface changes anywhere in this plan.
