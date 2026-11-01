# x86_32 Port Parity — build, run and test via `cargo xtask`

> **Status:** Executing. M1 (Gap 1 diagnosis) complete 2026-10-31 — see `plans/x86_32-gap1-diagnosis.md`
> for the contract diff and measured stall link. Written 2026-10-31 at worktree `clever-knight`, HEAD `c131b41`.
> **Goal:** bring the x86_32 port to the same level of quality as x86_64: buildable, runnable and
> integration-tested through `cargo xtask`, with CI guarding it.
> **Method:** every code fact below is cited to a path (and line) re-verified against this tree;
> every runtime fact was measured on this machine (QEMU 10.2.1, `nightly-2026-10-03`). Measured
> runtime observations are marked **[O]**. Facts this plan cannot prove are marked **unverified**
> and routed to the milestone that must prove them.
> Repo law applies throughout: `kernel/` stays `no_std` and arch-agnostic, **no comments ever**
> (assembly excepted), unit tests for kernel modules, no test-only production surface,
> `cargo build --workspace` stays forbidden, commit subjects are bare imperative (no prefix).

---

## 0. Goal & Non-Goals

### Goal
- `cargo xtask build [x86_64|x86_32]`, `run`, `apps`, `rom` build the x86_32 kernel + user ELFs +
  ROM image and boot it in QEMU with one command.
- The existing integration-test machinery (QEMU session, monitor `sendkey`, debugcon capture) runs
  against both arches, selected by environment; an x86_32 test subset runs in CI.
- The x86_32 runtime lineup matches x86_64's: ROM + `vfsd` + the IPC service roster + `atad` with
  port grants.
- CI builds x86_32 on every push (drift kill), later plus the integration subset.

### Non-Goals
- No in-house bootloader for x86_32 (the `bootloader` crate is x86_64-only — F11; writing a
  replacement is out of scope).
- No framebuffer/VGA-mode work on x86_32; VGA text + debugcon is the console contract there.
- No x86_32 gdb/debug scaffolding this iteration (x86_64's `debug-qemu.sh` family stays the model).
- No new third-party crates in `kernel/`; `xtask` and `tests-integration` keep their current deps.

---

## 1. Verified baseline facts

### Code facts (cited against `c131b41`)
- **F1** `xtask/src/main.rs` is hardcoded to x86_64 at every step: `USER_APPS` (line 5),
  `build_apps()` targets `arch/x86_64/rosx-user.json` (line 39), `build_kernel()` is
  `-p rosx --target arch/x86_64/rosx.json` (lines 50–51), `build_image()` shells into
  `arch/x86_64-runner/Cargo.toml` (lines 65–66), `pack_rom()` reads `target/rosx-user/release/*`
  and writes `target/rosx-user/rom.img` (lines 80–96), `run` rejects any arch but `x86_64`
  (lines 158–163).
- **F2** x86_32 kernel package is `rosx-x86` (`arch/x86_32/Cargo.toml`), custom targets
  `arch/x86_32/rosx-i686.json` (kernel) and `arch/x86_32/rosx-i686-user.json` (PIE user ELFs);
  root `.cargo/config.toml` already carries `[target.rosx-i686-user]` rustflags
  (`-Tlinker.ld`, `--pie`) — no config gap on the user side.
- **F3** `arch/x86_32/run.sh` boots `qemu-system-i386 -kernel <elf>` and lacks `-display none`;
  it passes `-d cpu_reset`. `build-image.sh` produces a GRUB multiboot ISO and needs
  `grub-mkrescue` + `xorriso` + `mtools`.
- **F4** `arch/x86_32/src/main.rs` schedules kstatsd (58), inputd (59–60), ps2d (61–62), shell
  (63–64). No `rom::init`, no `vfsd`, no `randomd`/`echo`/`bulk`/`pong`, no `atad`, no
  `portio::grant`. Boot banner is `[x86] Bootstrapped` (54). `unused_unsafe` warning at line 74.
- **F5** `arch/x86_64/src/main.rs` is the parity reference: `rom::init` from
  `target/rosx-user/rom.img` (78–79), kstatsd + randomd + echo + bulk + pong + vfsd + inputd +
  ps2d + atad with `portio::grant(0x1F0..0x1F7)` and `grant(0x3F6)` (80–110), shell (111–112);
  banner strings `[KERNEL] Initializing` (75) and `[KERNEL] Starting` (115).
- **F6** `arch/x86_32/build.rs` asserts only that `target/rosx-i686-user/release/shell` exists
  (lines 5–7); `arch/x86_64/build.rs` asserts the full embedded roster + `rom.img`.
- **F7** `tests-integration` is x86_64-shaped: `kernel_build.rs` hardcodes
  `arch/x86_64/Cargo.toml` + `rosx.json` (lines 32–33) and artifact `target/rosx/<profile>/rosx`
  (72–76); `qemu.rs` hardcodes `qemu-system-x86_64` (line 46); `disk_image.rs` is
  `bootloader::BiosBoot` (line 5). Escape hatches exist: `ROSX_KERNEL_ELF`, `ROSX_QEMU_PROFILE`.
- **F8** The harness core is arch-neutral: debugcon-to-stdout capture (`qemu.rs` 62–77), unix-socket
  monitor + `sendkey` (`sendkey.rs` token table incl. `' ' → spc`, line 49), output marking, temp-dir
  and teardown discipline. The scratch IDE disk is a plain `-drive if=ide` (50–60).
- **F9** `system::rom::encode` is arch-neutral byte packing (used by `pack_rom`, F1) — the i686 ROM
  packs with the same call.
- **F10** Preemption contract (kernel side): `preempt()` sets `pending_preemption`
  (`kernel/src/kernel.rs` 246–253); `apply_pending_preemption()` is the only IRQ-exit switch point
  (255–258). Commit `cf97d19` added the deferred switch to **both** arch timer handlers
  (x86_32 `interrupts.rs` 161–162 keeps it) but the "restore interrupts on syscall return" half
  changed only `arch/x86_64/src/syscall.S` — the x86_32 `int80_handler`
  (`arch/x86_32/src/cpu.rs` 163–185) never received the equivalent review.
- **F11** The `bootloader` crate 0.11.x is x86_64-only: every internal stage crate is
  `bootloader-x86_64-*` and its disk-image step forces objcopy `--binary-architecture=i386:x86-64`.
  There is no drop-in `BiosBoot` for i686.

### Measured runtime facts **[O]** (this machine, 2026-10-31)
- **O1** All 19 `USER_APPS` + `shell` compile for `rosx-i686-user` (release, ~29 s, warnings only).
  The x86_32 userland port is done.
- **O2** `rosx-x86` compiles for `rosx-i686` from the workspace root with the standard `-Z` trio;
  the port is current with the `c131b41` kernel API surface (only F4's `unused_unsafe`).
- **O3** `qemu-system-i386 -kernel target/rosx-i686/debug/rosx-x86 -debugcon stdio -display none`
  boots: multiboot v1 header accepted, mmap parsed (`[MEMORY] Region …`), kstatsd/inputd/ps2d/shell
  banners, `rose>` prompt. Without `-display none` it dies headless (`gtk initialization failed`).
- **O4** At the prompt `ls` lists the five builtins and then prints `No filesystem: NoConnection`
  (no vfsd — consistent with F4).
- **O5** With QEMU `-d int`: timer `v=20` ~100/s, keyboard `v=21` exactly per injected key
  (make+break), syscall `v=80` in bursts — all three interrupt classes are delivered and handled.
- **O6** **Keystroke echo is unreliable.** Identical stimulus (monitor `sendkey`, letters at
  0.4–0.5 s spacing after the prompt is up): run 1 echoed 6/6, run 2 echoed only the first of 6,
  runs 3–5 echoed 0/10. Every injected scancode produced its `v=21` (O5), so the loss is in the
  post-handler wake chain `deliver_to_service → ps2d → inputd → shell`, not in delivery to the kernel.
- **O7** GRUB image toolchain on this machine: `grub-mkrescue` present, `xorriso` and `mtools`
  **missing** → the GRUB image path cannot run here today; CI would need an apt install step.

---

## 2. Gap 1 — Input-delivery reliability (PREREQUISITE, fix first)

**Symptom [O6]:** typed keys reach the kernel (every `v=21` accounted for) but the shell echoes
0, 1, or all of a burst — the chain stalls nondeterministically right after boot.

**Why first:** every interactive integration test (echo, commands, lifecycle) needs this; the fix is
arch-scoped with near-zero x86_64 risk; and if the root cause turns out to live in the shared wake
path, that reshapes everything downstream — better known before tooling lands on top of it.

**Suspects (ordered):**
1. **`int80_handler` interrupt-window contract (F10).** The x86_32 trap gate leaves `IF` as the
   caller had it; `handle_syscall` park paths rely on the arch to control the interrupt window
   between "future registered" and "context switched away". `swap_context` (`cpu.rs` 126–147)
   does `cli … sti; ret` — a park that registers a waiter with `IF=1` (or never re-enables on a
   path the `cf97d19` fix corrected on x86_64) admits a wake that lands in the window and is lost.
   This is the exact bug class `cf97d19` eliminated on x86_64 and the half that never touched the
   x86_32 syscall path.
2. **IRQ-context handoff:** x86_32 keyboard handler (interrupts.rs 165–173) does
   `in 0x60; EOI; deliver_to_service` with `IF=0` for its whole duration, while the wake is only
   serviced at the next timer tick — combined with suspect 1 the window arithmetic differs from
   x86_64's `dispatch_irq` exit path.
3. **Idle/halt wake:** whether `Cpu::halt` + timer tick re-enters `apply_pending_preemption` in all
   states (lower priority — the 100 Hz tick makes a permanent stall here unlikely).

**Diagnostic protocol (instrument, don't guess):**
- D1. Side-by-side contract diff: `arch/x86_64/src/syscall.S` vs x86_32 `int80_handler` +
  `swap_context` + timer/keyboard handlers, annotated with the `cf97d19` invariant: *interrupts are
  disabled across park-registration and restored by the arch exit path; preemption is applied only
  at IRQ exit*. Output: a written contract statement + the specific violation(s).
- D2. Temporary kernel-side wake-path counters (keystrokes delivered, ps2d accept wakes,
  inputd wakes, shell wakes) emitted on panic/quit or periodically from kstatsd — throwaway,
  removed before the fix commit lands. Run the sendkey loop, read off the link that stops waking.
- D3. Fix in the arch layer aligned to the x86_64 contract. If D2 indicts shared `kernel/` wake
  code, **stop and escalate**: that is a kernel bug with x86_64 blast radius; it gets its own
  plan section and the full x86_64 suite gates it.

**Acceptance gate (fix commit):** scripted loop — 10 consecutive VM boots, 10-key burst each,
every run 10/10 echoed (the script becomes the seed of the M6 harness test). Plus
`cargo xtask test --skip-integration` green and the full x86_64 integration suite green.

---

## 3. Gap 2 — Arch-aware xtask

Add an arch registry in `xtask` (new `xtask/src/arch.rs`): `ArchSpec { name, kernel_package,
kernel_target_spec, user_target_spec, kernel_dir_name, kernel_binary, user_dir_name, rom_path,
qemu_binary }` with two instances — `x86_64` (values from F1) and `x86_32` (`rosx-x86`,
`arch/x86_32/rosx-i686.json`, `arch/x86_32/rosx-i686-user.json`, `target/rosx-i686`, binary
`rosx-x86`, `target/rosx-i686-user`, `qemu-system-i386`).

- `build_apps` / `pack_rom` / `build_kernel` / `build_image` take `&ArchSpec` unchanged in shape;
  `USER_APPS` list is shared (O1 proves it compiles for both).
- Subcommands gain an optional positional arch: `cargo xtask <build|apps|rom|run> [x86_64|x86_32]`,
  default `x86_64`, unknown arch → usage + exit 2. `run`'s current hard rejection (F1) becomes the
  registry lookup. `test` keeps its current shape (host unit tests are arch-neutral); the
  integration leg gets its arch from `ROSX_ARCH` in M6, not from xtask.
- x86_32 `build`/`run` boot via Gap 3's runner instead of `x86_64-runner`.

**Gate:** `cargo xtask build` (x86_64) produces the identical artifact set as before; `cargo xtask
build x86_32` compiles apps → ROM (`target/rosx-i686-user/rom.img`) → kernel → bootable artifact;
full x86_64 integration suite green (run_tests consumes the refactored functions).

---

## 4. Gap 3 — Run & image path

**DECIDED (owner, 2026-10-31): multiboot.** The canonical x86_32 boot artifact is the QEMU
`-kernel` multiboot ELF; the GRUB ISO stays an optional real-hardware path outside CI.
- **A (CHOSEN): QEMU `-kernel` multiboot ELF** (current `run.sh` mechanics — F3, proven by
  O3). Zero host dependencies; QEMU-only.
- B: GRUB multiboot ISO via `build-image.sh`. Real-hardware capable but host-dep heavy; **O7** shows
  the toolchain is missing even on this dev machine. Kept reachable as `cargo xtask image x86_32`,
  never a CI requirement.

Implementation:
- Fix `run.sh`: add `-display none`; drop `-d cpu_reset` (register-dump flood; the integration
  harness deliberately never passed it). Keep it compatible with both launch routes (cargo runner
  config in `arch/x86_32/.cargo/config.toml` and xtask's direct invocation).
- `xtask run x86_32` = build kernel → spawn `qemu-system-i386 -kernel <elf>` with the same flags
  the x86_64 runner uses (`-debugcon stdio -no-reboot -no-shutdown -display none`).
- `build-image.sh` stays as the optional real-hardware path, reachable as
  `cargo xtask image x86_32` — probes `grub-mkrescue`/`xorriso`/`mtools` and reports the missing
  set instead of failing deep inside the build. **unverified** whether the GRUB ISO still boots at
  `c131b41` (toolchain gap); the milestone that first runs it must prove it.

---

## 5. Gap 4 — Runtime lineup parity (x86_64 boot roster, F5)

- **ROM:** `cargo xtask rom x86_32` packs `target/rosx-i686-user/rom.img` (F9); kernel embeds it
  via `include_bytes!` + `kernel::rom::init` (mirror x86_64 `main.rs` 78–79). This is what makes
  shell-by-name execution (`rose> snake`) work on x86_32.
- **Roster:** schedule `randomd`, `echo`, `bulk`, `pong`, `vfsd` (boot order per F5), keeping
  inputd/ps2d/kstatsd/shell where they are.
- **`atad` + grants:** schedule atad with `portio::grant(0x1F0..0x1F7)` + `grant(0x3F6)` exactly as
  F5; x86_32 already wires the mediation mechanism (AGENTS.md: boots with no grants today).
- **Banners:** emit the same boot markers as x86_64 — `[KERNEL] Initializing` / `[KERNEL] Starting`
  in `arch/x86_32/src/main.rs` (replace `[x86] Bootstrapped`) — so shared assertions hold.
- **build.rs:** extend `arch/x86_32/build.rs` (F6) to assert the whole embedded roster + ROM image,
  mirroring F5's x86_64 checks.

**Gate:** `cargo xtask run x86_32` reaches the prompt; `ls` lists ROM apps instead of
`No filesystem: NoConnection` (O4 flips); `mem` runs from ROM against kstatsd; typed `snake`
launches from ROM. x86_64 untouched; full suite green.

---

## 6. Gap 5 — tests-integration arch parameterization

- **Arch selection:** `ROSX_ARCH` env var (`x86_64` default | `x86_32`) read by `kernel_build` and
  `qemu`. `kernel_build` swaps manifest/target/artifact per F7's table (keeps `ROSX_KERNEL_ELF`
  and `ROSX_QEMU_PROFILE` semantics). `qemu.rs` swaps the binary per arch and abstracts the boot
  mode: BIOS disk drive for x86_64 (unchanged), `-kernel <elf>` for x86_32 (Gap 3A). Monitor
  socket, debugcon capture, sendkey, temp dirs, teardown: untouched (F8).
- **Ported subset 1** (needs Gaps 1+3+5 plus Gap 4's banner unification): `kernel_boots`,
  `shell_banner_and_prompt`, `shell_echoes_keystrokes` — as x86_32 variants of the existing test
  bodies where assertions are
  already arch-neutral, arch-gated otherwise.
- **Ported subset 2** (needs Gap 4): `shell_ls_and_unknown_command`, `ipc_random`, `ipc_bulk`,
  `ipc_ping`, `kstats_mem`.
- **Ported subset 3** (last): `elf_arguments`, `shell_command_arguments`, `app_lifecycle`,
  `waitset_or`, `stress_mixed_preemption`; then the five `ata_*` (scratch `-drive if=ide` is plain
  QEMU — **unverified** on `qemu-system-i386`, proved when the first ata test is x86_32-enabled).
- x86_32 tests live in the same files, gated on `ROSX_ARCH`, so `cargo test -p tests-integration`
  (host default x86_64) is unaffected; the x86_32 leg runs by setting the env var.

**Gate:** `ROSX_ARCH=x86_32 cargo test -p tests-integration -- --test-threads=1` green for the
ported subsets; default invocation byte-identical in behavior for x86_64.

---

## 7. Gap 6 — CI

- `ci.yml`: new `build-x86_32` job (same cache shape as `build`), step
  `cargo xtask build x86_32`, upload `target/rosx-i686/release/rosx-x86` (+ `rom.img`; the ISO only
  if/when path B is proven). This alone kills the drift that today only maintainer discipline prevents.
- After Gap 5 subset 1 lands: `test-x86_32-integration` job (ubuntu runners ship `qemu-system-i386`
  — **unverified** for the pinned runner image, proved on first workflow run),
  `ROSX_ARCH=x86_32 cargo test -p tests-integration …`.
- `build-artifacts.yml`: mirror the x86_64 job for x86_32.

**Gate:** a green push to a PR branch shows all jobs; no new required apt packages (the multiboot
decision of 2026-10-31 leaves nothing outside Cargo + QEMU, which is gap-free by construction on
GH runners).

---

## 8. Gap 7 — Hygiene (fold into the commits that touch each area)

- Fix `unused_unsafe` at `arch/x86_32/src/main.rs:74` (lands with Gap 1's file touches).
- Rename decision (owner): package `rosx-x86` → `rosx-x86_32` for clarity against `rosx`; cosmetic,
  touches Cargo.tomls + xtask registry + CI upload paths. Only if approved; nothing functional.
- Update `AGENTS.md` build/run/test sections and `README.md` to the new xtask recipes; retire the
  "build it with explicit `-Z` flags by hand" paragraph for x86_32.
- x86_32 debug scaffolding (gdb/QEMU-background equivalents of x86_64's four scripts): explicitly
  deferred to a follow-up plan.

---

## 9. Milestones (each = one reviewable commit or commit pair)

| # | Content | Depends on | Gate |
|---|---|---|---|
| M1 | Gap 1 diagnosis artifact (contract diff + instrumented findings, committed as a note in this file's follow-up or plans/) | — | Stall link identified |
| M2 | Gap 1 fix + 10×10 echo stress script | M1 | O6 flips to 10/10 ×10 runs |
| M3 | Gap 2 arch registry + `build/apps/rom x86_32` | — | both arches build; x86_64 suite green |
| M4 | Gap 3 run.sh fix + `xtask run x86_32` (+ optional `image`) | M3 | prompt reached via xtask |
| M5 | Gap 4 ROM + roster + atad/grants + banners + build.rs | M3, M4 | O4 flips to ROM listing; `snake` from prompt |
| M6 | Gap 5 harness arch knob + subset 1 | M2, M5 | `ROSX_ARCH=x86_32` subset-1 green |
| M7 | Gap 6 CI build job (+ artifacts) | M3 | green CI x86_32 job |
| M8 | Gap 5 subsets 2–3 + CI integration job | M5, M6 | ported suites green locally, then in CI |
| M9 | Gap 7 docs/naming | all | AGENTS.md/README current |

**Fallback rule (agreed 2026-10-31):** M1/M2 sit on the critical path for M6/M8 only. If M1 turns
into a multi-day hunt, M3–M5 and M7 are independent and land first; the x86_32 integration leg stays
boot-only tests until M2 lands. Never land an interactive x86_32 test that is `#[ignore]`d or flaky.

**Regression discipline for every milestone:** `cargo xtask test --skip-integration` plus the full
x86_64 integration suite stay green; shared-code edits (xtask, harness, CI) are gated by the
x86_64 pipeline specifically, since it is the known-good reference.
