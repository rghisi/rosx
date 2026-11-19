---
priority: normal
date: 2026-11-15
trigger: none
---

# Harness gate: a disk-booted session must prove its mount

`QemuSession` accepts `rose>` as proof of boot, but a session reaches a prompt even when `run rom:mount ata0d0p0 /` fails: log-and-continue marks the volume-sourced services `not available` and the ROM shell takes over with an empty mount table. Tests then fail with `Unknown command: <name>`, pointing at the test instead of the boot. Observed once on CI (PR #169, `shell_background` x86_64; the identical commit passed on rerun).

Add `QemuSession::expect_disk_boot()` that waits for the mount tool's marker (`mount ok device=ata0d0p0 dir=/`) before any input is typed, called by every suite that resolves binaries through the root volume. The ROM fallback stays legal; `disk_boot`'s own tests keep asserting the hand-off explicitly.
