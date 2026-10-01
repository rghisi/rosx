# Unified Build — `cargo xtask` Single-Command Build & Test

> **Status:** Planning (not started)
> **Scope:** Build system only. No kernel/system/usrlib logic changes. No behavior changes to any binary.
> **Platform focus:** x86_64 only. Do NOT touch `arch/x86_32` build inputs (it must keep compiling as-is).
> **Goal:** From the repo root, `cargo xtask build` builds all user ELF apps + the x86_64 kernel + the bootable disk image, and `cargo xtask test` runs every unit test in the workspace. Plain `cargo build` / `cargo test` at the root must also succeed (host-checkable crates only).

---

## 1. Purpose

Today the build is fragmented:

- `cargo build` at the repo root **fails**: `arch/x86_64` (`rosx`) and `arch/x86_32` (`rosx-x86`) are workspace members but require custom target specs + `build-std`, configured via per-directory `.cargo/config.toml`. Cargo resolves config from CWD, not from the package location, so from the root these bare-metal bins get compiled for the host and fail (`error: instruction requires: Not 64-bit mode`).
- The 5 user ELF apps (`hello_elf`, `random_gen_server`, `snake`, `tetris`, `conway`) are **excluded** from the workspace. Each is its own mini-workspace with its own `Cargo.lock`, its own `target/`, and a `.cargo/config.toml` injecting `-T<linker.ld> --pie` rustflags with fragile paths relative to the repo root.
- CI (`.github/workflows/ci.yml`, `build-artifacts.yml`) hand-orders 6 separate `cargo build` invocations because cargo cannot express the dependency "build the app ELFs before the kernel embeds them".

A single native `cargo build` invocation **cannot** do this: one invocation = one config + target selection applied to every selected package, and the kernel needs `rosx.json` (non-PIE) while the apps need `rosx-user.json` (PIE + linker script). Cargo aliases cannot chain commands. The idiomatic solution is an **xtask** crate: a std workspace member that orchestrates cargo subprocesses in the correct order.

### Locked decisions (do not re-litigate)

| # | Decision | Choice |
|---|----------|--------|
| 1 | Single-command mechanism | **`cargo xtask build` / `cargo xtask test`** via a root cargo alias. |
| 2 | Arch coverage | **x86_64 only.** `arch/x86_32` untouched (must still build via `cd arch/x86_32 && cargo build`). |
| 3 | Disk images | **Always.** `cargo xtask build` always produces `target/rosx/<profile>/rosx-x86_64.img`. |
| 4 | App set | **Build all 5 apps** even though only `random_gen_server` is embedded by the kernel today. |
| 5 | Test scope | Unit tests now; `--integration` flag is a reserved no-op hook (see §6.3). `tools/integration-tests/` contains no code yet. |
| 6 | Apps join the workspace | Yes: single root `Cargo.lock`, single root `target/`. Per-app linker flags move from `.cargo/config.toml` rustflags into a tiny per-app `build.rs` (same pattern as `arch/x86_32/build.rs`). |

---

## 2. As-Is Snapshot (verified 2026-10-01 — re-verify before starting, line numbers drift)

### 2.1 Workspace

- Root `Cargo.toml`: virtual workspace. `members = [collections, system, kernel, usrlib, arch/x86_64, arch/x86_32, apps/shell, apps/dummy]`, `exclude = [apps/hello_elf, apps/random_gen_server, apps/snake, apps/tetris, apps/conway]`. `apps/test_suite` is an implicit member (path-dep of `shell` and `rosx`). Edition 2024, `panic = "abort"`.
- No `.cargo/config.toml` at the repo root.
- `arch/x86_64-runner/` is its **own** workspace (has `[workspace]` in its `Cargo.toml`) — leave it alone.

### 2.2 Cross-build configs

- `arch/x86_64/.cargo/config.toml`: `build.target = "rosx.json"`, `[unstable] build-std = ["core","alloc","compiler_builtins"]`, `build-std-features = ["compiler-builtins-mem"]`, `json-target-spec = true`, runner = `./run.sh`.
- `arch/x86_32/.cargo/config.toml`: same shape with `rosx-i686.json`.
- Each of the 5 apps has `apps/<app>/.cargo/config.toml`: `build.target = "../../arch/x86_64/rosx-user.json"`, same `[unstable]` block, plus rustflags `["-C","link-arg=-T../../apps/<app>/linker.ld","-C","link-arg=--pie"]` under both `cfg(target_arch = "x86_64")` and `cfg(target_arch = "x86")`.
- **All 5 `apps/<app>/linker.ld` files are byte-identical** (md5 `a9dd9c81247df698fa17872f2ad9fb82`).
- Each app has its own `Cargo.lock` and `target/` (git-ignored).
- All 5 apps are `#![no_std] #![no_main]` with `pub extern "C" fn _start()` and their own `#[panic_handler]` — they **cannot** compile or test on the host.

### 2.3 Kernel embedding

- `arch/x86_64/src/main.rs:74`: `include_bytes!("../../../apps/random_gen_server/target/rosx-user/release/random_gen_server")` — an implicit build-order dependency cargo cannot see. The other 4 ELFs are built in CI but embedded nowhere.
- `arch/x86_32` embeds no app ELFs. `arch/x86_32/build.rs` already uses the `cargo::rustc-link-arg=-T{dir}/linker.ld` pattern.

### 2.4 Verified current behaviors

- `cargo build` at root: **FAILS** (host-compiles `rosx`/`rosx-x86` bins).
- `cargo test --workspace -- --test-threads=1` at root: **PASSES** (188 tests: kernel 168, collections 15, shell 5; arch bins are skipped because `[[bin]] test = false`).
- `cargo build -p kernel` on host: **PASSES** (no_std rlib compiles fine on host).
- CI test job runs only `cargo test -p collections` + `cargo test -p kernel -- --test-threads=1`.
- Disk image: `cargo run --manifest-path arch/x86_64-runner/Cargo.toml -- target/rosx/release/rosx x86_64 --no-run` → `target/rosx/release/rosx-x86_64.img`.
- `kernel/.cargo/config.toml` contains a bogus `[target.'cfg(test)'.dependencies] std = ...` block (invalid config key, silently ignored) — optional cleanup, not required.

---

## 3. To-Be End State

| Command (from repo root) | Result |
|---|---|
| `cargo xtask build` | 1) all 5 app ELFs → `target/rosx-user/release/` 2) kernel → `target/rosx/release/rosx` 3) disk image → `target/rosx/release/rosx-x86_64.img` |
| `cargo xtask build --debug` | Same, kernel+image in `target/rosx/debug/` (apps stay `--release`; the embedded ELF path is release-only, as today) |
| `cargo xtask apps` | Only step 1 (useful before `cargo run` inside `arch/x86_64`) |
| `cargo xtask test` | `cargo test --workspace -- --test-threads=1` (all host unit tests) |
| `cargo build` (root) | Builds host-checkable libs only (`default-members`), succeeds |
| `cargo test` (root) | Runs all host unit tests, succeeds |
| `cd arch/x86_64 && cargo run` | Still works (needs `cargo xtask apps` run once so the embedded ELF exists) |
| `cd arch/x86_32 && cargo build` | Still works, untouched |

`cargo build --workspace` will **never** work (bare-metal bins + PIE apps cannot compile for the host). This is documented, not fixed.

---

## 4. Execution Steps

Work incrementally. Each step ends with its verification green and its own commit (commit messages: plain sentences, no prefix — see AGENTS.md).

### Step 1 — Merge the 5 ELF apps into the root workspace

**Files:** root `Cargo.toml`, `apps/{hello_elf,random_gen_server,snake,tetris,conway}/Cargo.toml`, new `apps/<app>/build.rs`, delete `apps/<app>/.cargo/config.toml`, delete `apps/<app>/Cargo.lock`.

1. Root `Cargo.toml`: add the 5 app paths to `members`; delete the entire `exclude` table.
2. In each app `Cargo.toml`, add (adjust `name` to the package name) and switch `edition = "2024"` → `edition.workspace = true`:

   ```toml
   [[bin]]
   name = "hello_elf"
   bench = false
   test = false
   ```

   `test = false` is what keeps them out of `cargo test --workspace` (they are `#![no_main]` and cannot host-compile).
3. Create `apps/<app>/build.rs` (identical in all 5, mirrors `arch/x86_32/build.rs`):

   ```rust
   fn main() {
       let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
       println!("cargo::rustc-link-arg=-T{dir}/linker.ld");
       println!("cargo::rustc-link-arg=--pie");
   }
   ```

4. `git rm` the 5 `apps/<app>/.cargo/config.toml` and the 5 `apps/<app>/Cargo.lock`.
5. Keep the per-app `linker.ld` files where they are.

**Verify:**

```bash
cargo build --release -p hello_elf -p random_gen_server -p snake -p tetris -p conway \
  --target arch/x86_64/rosx-user.json \
  -Zbuild-std=core,alloc,compiler_builtins \
  -Zbuild-std-features=compiler-builtins-mem \
  -Zjson-target-spec
```

Expect all 5 ELFs in `target/rosx-user/release/` (the target name is the JSON file stem, `rosx-user`). Spot-check one is a PIE ET_DYN: `readelf -h target/rosx-user/release/hello_elf | grep Type`. Then `cargo test --workspace -- --test-threads=1` still green.

**Commit:** `Build user ELF apps as workspace members with per-app linker script build scripts`

### Step 2 — Repoint the kernel's embedded ELF and track app changes

**Files:** `arch/x86_64/src/main.rs`, new `arch/x86_64/build.rs`.

1. `arch/x86_64/src/main.rs:74` — change the `include_bytes!` path to the unified workspace target dir:

   ```rust
   static RANDOM_GEN_SERVER_ELF: &[u8] =
       include_bytes!("../../../target/rosx-user/release/random_gen_server");
   ```

2. New `arch/x86_64/build.rs` — rerun the kernel build when the app changes, and fail with a clear message instead of "couldn't read" when the ELF is missing:

   ```rust
   use std::path::Path;

   fn main() {
       println!("cargo::rerun-if-changed=../../apps/random_gen_server/src");
       println!("cargo::rerun-if-changed=../../apps/random_gen_server/Cargo.toml");
       println!("cargo::rerun-if-changed=../../apps/random_gen_server/linker.ld");
       let elf = Path::new("../../target/rosx-user/release/random_gen_server");
       if !elf.exists() {
           panic!("missing user ELF {}; run `cargo xtask apps` first", elf.display());
       }
   }
   ```

   Paths are relative to the package dir (`arch/x86_64`). **Do NOT build the apps from this build script** — a nested `cargo build` sharing the same target dir can deadlock on cargo's build-dir lock. Ordering is xtask's job (Step 3).

**Verify:**

```bash
cd arch/x86_64 && cargo build --release        # succeeds (ELF exists from Step 1 verify)
touch apps/random_gen_server/src/main.rs
cd arch/x86_64 && cargo build --release        # kernel rebuilds (build script reran)
```

Then temporarily `mv target/rosx-user/release/random_gen_server /tmp/` and confirm the build fails with the `cargo xtask apps` message; restore the file.

**Commit:** `Embed random_gen_server from unified target dir and rerun kernel build on app changes`

### Step 3 — Add the `xtask` crate and alias

**Files:** new `xtask/Cargo.toml`, new `xtask/src/main.rs`, new root `.cargo/config.toml`.

1. `xtask/Cargo.toml`:

   ```toml
   [package]
   name = "xtask"
   publish = false
   version = "0.1.0"
   edition.workspace = true
   ```

   No dependencies (std only). Add `"xtask"` to root `members`.
2. Root `.cargo/config.toml` — **aliases only**. Any `[build]` key here would leak into `arch/x86_64` / `arch/x86_32` builds (config merges upward) and would break `cargo test` (tests need the host target):

   ```toml
   [alias]
   xtask = ["run", "--package", "xtask", "--"]
   ```

3. `xtask/src/main.rs` — reference implementation (adapt freely; keep it comment-free per AGENTS.md):

   ```rust
   use std::env;
   use std::path::PathBuf;
   use std::process::{self, Command};

   const USER_APPS: [&str; 5] = ["hello_elf", "random_gen_server", "snake", "tetris", "conway"];

   const UNSTABLE: [&str; 3] = [
       "-Zbuild-std=core,alloc,compiler_builtins",
       "-Zbuild-std-features=compiler-builtins-mem",
       "-Zjson-target-spec",
   ];

   fn workspace_root() -> PathBuf {
       PathBuf::from(env!("CARGO_MANIFEST_DIR"))
           .parent()
           .unwrap()
           .to_path_buf()
   }

   fn cargo() -> Command {
       let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
       cmd.current_dir(workspace_root());
       cmd
   }

   fn run(cmd: &mut Command, description: &str) {
       println!("==> {description}");
       let status = cmd.status().expect("failed to spawn cargo");
       if !status.success() {
           eprintln!("failed: {description}");
           process::exit(status.code().unwrap_or(1));
       }
   }

   fn build_apps() {
       let mut cmd = cargo();
       cmd.arg("build").arg("--release");
       for app in USER_APPS {
           cmd.arg("-p").arg(app);
       }
       cmd.arg("--target").arg("arch/x86_64/rosx-user.json");
       cmd.args(UNSTABLE);
       run(&mut cmd, "build user-space ELF apps");
   }

   fn build_kernel(debug: bool) {
       let mut cmd = cargo();
       cmd.arg("build");
       if !debug {
           cmd.arg("--release");
       }
       cmd.arg("-p").arg("rosx");
       cmd.arg("--target").arg("arch/x86_64/rosx.json");
       cmd.args(UNSTABLE);
       run(&mut cmd, "build x86_64 kernel");
   }

   fn build_image(debug: bool) {
       let profile = if debug { "debug" } else { "release" };
       let kernel = workspace_root()
           .join("target")
           .join("rosx")
           .join(profile)
           .join("rosx");
       let mut cmd = cargo();
       cmd.arg("run")
           .arg("--manifest-path")
           .arg("arch/x86_64-runner/Cargo.toml")
           .arg("--")
           .arg(&kernel)
           .arg("x86_64")
           .arg("--no-run");
       run(&mut cmd, "create x86_64 disk image");
   }

   fn run_tests(integration: bool) {
       let mut cmd = cargo();
       cmd.arg("test")
           .arg("--workspace")
           .arg("--")
           .arg("--test-threads=1");
       run(&mut cmd, "run all unit tests");
       if integration {
           eprintln!("warning: integration tests are not implemented yet; skipping");
       }
   }

   fn usage() {
       eprintln!("usage: cargo xtask <build [--debug] | apps | test [--integration]>");
   }

   fn main() {
       let args: Vec<String> = env::args().skip(1).collect();
       let subcommand = args.first().map(String::as_str).unwrap_or("build");
       let debug = args.iter().any(|a| a == "--debug");
       match subcommand {
           "apps" => build_apps(),
           "build" => {
               build_apps();
               build_kernel(debug);
               build_image(debug);
           }
           "test" => run_tests(args.iter().any(|a| a == "--integration")),
           "help" | "--help" | "-h" => usage(),
           other => {
               eprintln!("unknown subcommand: {other}");
               usage();
               process::exit(2);
           }
       }
   }
   ```

   Notes:
   - Apps are always built `--release`: the `include_bytes!` path is release-only (unchanged behavior; kernel `--debug` embeds the release ELF, exactly like today).
   - `-Z` flags require nightly — already pinned by `rust-toolchain.toml`.
   - `CARGO` env var is set automatically by the `cargo xtask` alias; the fallback covers `cargo run -p xtask` oddities.
   - Order is load-bearing: apps → kernel → image.

**Verify:**

```bash
cargo xtask build          # ends with target/rosx/release/rosx-x86_64.img newer than the run started
cargo xtask test           # 188+ tests green
cargo xtask apps           # ELFs present
```

Boot check: `cd arch/x86_64 && cargo run` (QEMU opens; shell prompt and random_gen_server behavior unchanged). Exit QEMU to confirm clean exit code.

**Commit:** `Add xtask crate for single-command build and test orchestration`

### Step 4 — Make plain root `cargo build` / `cargo test` succeed

**Files:** root `Cargo.toml`.

Add `default-members` (everything host-buildable; excludes the two arch bins and the 5 PIE apps):

```toml
default-members = [
    "collections",
    "system",
    "kernel",
    "usrlib",
    "apps/shell",
    "apps/dummy",
    "apps/test_suite",
    "xtask",
]
```

**Verify:**

```bash
cargo build                # green (host libs only)
cargo test                 # green, runs kernel/collections/shell tests
cargo test --workspace     # same coverage as before, still green
```

**Commit:** `Set workspace default members so plain cargo build and test succeed at the root`

### Step 5 — CI and docs

**Files:** `.github/workflows/ci.yml`, `.github/workflows/build-artifacts.yml`, `AGENTS.md`, `README.md`.

1. `ci.yml` test job: replace the two test steps with `cargo xtask test`.
2. `ci.yml` build job + `build-artifacts.yml`: replace the 5 app-build steps + kernel build step + disk-image step with a single `cargo xtask build` (x86_64 jobs only; leave the x86_32 job exactly as-is). Artifact upload paths unchanged (`target/rosx/release/rosx-x86_64.img`).
3. Remove the now-dead `apps/*/target/` cache paths from both workflows' `actions/cache` blocks.
4. Update the "Build & Run" section of `AGENTS.md` (and `README.md` if it documents builds) to lead with `cargo xtask build` / `cargo xtask test`, keep the per-arch recipes, and note that `cargo build --workspace` is intentionally unsupported.

**Verify:** `act` is not required; validate YAML by inspection + `cargo xtask build` / `cargo xtask test` locally reproducing exactly what CI runs.

**Commit:** `Use xtask in CI and document the unified build commands`

### Step 6 — Optional cleanup (separate commit, skip if in doubt)

- `git rm kernel/.cargo/config.toml` (bogus ignored config block, §2.4).
- `rm -rf apps/*/target` (dead dirs, git-ignored, reclaim disk).

---

## 5. Gotchas

1. **Root `.cargo/config.toml` must contain ONLY `[alias]`.** A `[build] target`/`rustflags`/`unstable` key there merges into every sub-build and breaks `cargo test` (host tests) and the arch configs.
2. **Never build the apps from `arch/x86_64/build.rs`** (nested cargo + shared target dir = lock deadlock risk). build.rs only checks existence + rerun-if-changed; xtask orders the real builds.
3. `cargo build --workspace` fails by design (host cannot link `#![no_main]`/`_start` PIE apps or bare-metal kernels). Only `cargo test --workspace` is safe, thanks to `test = false` on those bins.
4. Custom-target output dir = JSON file stem: `target/rosx/...` and `target/rosx-user/...`.
5. `include_bytes!` is hardcoded to `release` — a debug kernel embeds the release app ELF. Same as today; do not "fix" it here.
6. `arch/x86_64-runner` is a separate cargo workspace; invoke it only via `--manifest-path` (or its own dir).
7. Root `Cargo.lock` will churn in Step 1 (5 packages merged). Expected.
8. `-Zjson-target-spec` (CLI) == `json-target-spec = true` (config `[unstable]`). Nightly required; pinned.
9. AGENTS.md rules apply: **no comments in Rust code**, work in small steps, commit messages without prefixes, `kernel/` stays platform-agnostic (nothing here touches it).
10. `tools/integration-tests/` and `tools/ai-interface/` are untracked dirs containing only stale `target/` folders — do not reference them as existing tests.

---

## 6. Acceptance Criteria

1. `cargo xtask build` from a clean `target/` produces, in order: 5 ELFs in `target/rosx-user/release/`, `target/rosx/release/rosx`, `target/rosx/release/rosx-x86_64.img`.
2. `cargo xtask test` runs all workspace unit tests green (≥188 tests, kernel with `--test-threads=1`).
3. `cargo build` and `cargo test` at the root succeed (host libs).
4. `cd arch/x86_64 && cargo run` boots QEMU with the shell and random_gen_server, unchanged behavior.
5. `cd arch/x86_32 && cargo build` succeeds (untouched).
6. No `apps/*/.cargo/`, no `apps/*/Cargo.lock`, no per-app rustflags anywhere; exactly one root `Cargo.lock`.
7. CI workflows reduced to: test job = `cargo xtask test`; x86_64 build job = `cargo xtask build` + upload artifact.

### 6.3 Integration-test hook (future)

`cargo xtask test --integration` currently prints a warning and exits 0. When real QEMU-based integration tests land (e.g. under `tools/integration-tests/` as a workspace member or script), wire them into `run_tests()` behind that flag: build image first (`build_apps` + `build_kernel` + `build_image`), then drive QEMU headless (`-display none -serial stdio`) and assert on serial output.