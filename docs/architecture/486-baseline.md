# 486 baseline (x86_32)

## Invariants

- The x86_32 port runs on 486-class CPUs and stays that way; the pinned feature strings are the floor and are asserted by `cargo xtask check-arch`.
- No floating point on any 32-bit target; no `cpuid`-probed features; atomics of at most 32 bits.
- `cargo xtask check-isa x86_32` and `--stages`, and `-cpu 486` in every x86_32 QEMU session, are the machine guards: a regression must fail one of them, never a board.

## Detail
The x86_32 port runs on 486-class CPUs and must stay that way — the invariant, not the bug, is the default:

- **The pinned feature strings are the floor.** `arch/x86_32/rosx-i486.json` and `rosx-i486-user.json` compile against `-mmx,-sse,-sse2,-cmov,-fxsr,-cx8`; never widen them. `-cx8` pins `cmpxchg8b` out to keep the 486SX door open. Note: `RUSTFLAGS` in the environment *replaces* the `[target.rosx-*-user]` rustflags in `.cargo/config.toml` (which carry the `linker.ld` script) — never build user ELFs with an exported `RUSTFLAGS`.
- **No floating point on any 32-bit target** — kernel, usrlib, or apps. `swap_context` saves GPRs only, so any FP state would silently corrupt across task switches, and a 486SX has no FPU at all.
- **No `cpuid`-probed features** — `cpuid` itself is 586-era; the port runs flat protected mode with 386-era mechanism only (no paging, no CR4 bits).
- **Keep atomics ≤ 32 bits** on 32-bit paths; an `AtomicU64` there needs `cmpxchg8b`.
- **The machine guards:** `cargo xtask check-isa x86_32` audits the kernel and every user ELF against a 486 mnemonic allow-list, `cargo xtask check-isa bootloader` audits the stage-chain build and every `cargo xtask build x86_32` re-audits the flat stage1/stage2-32 binaries against a post-486 denylist (CI runs these in `build-x86_32` and `bootloader`), and every x86_32 QEMU session boots `-cpu 486`. A regression must fail one of these, never a board.
- **The two byte-gated allow-list exceptions:** `tzcnt` passes only in its legacy `f3`-prefixed form (executes as 386 `BSF`), and `ud2` passes only with the exact reserved `0f 0b` encoding — that opcode raises #UD identically on 486 silicon (Intel chose it for exactly that reason), and the compiler emits it only as a trap after a no-return panic path. Any other bytes decoding as `tzcnt`/`ud2` still fail the audit.
- Provenance: `plans/x86_32-i486-plan.md` and the evidence log `plans/x86_32-i486-impl.md`.
