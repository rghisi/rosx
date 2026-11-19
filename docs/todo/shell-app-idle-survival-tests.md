---
priority: normal
date: 2026-11-07
trigger: none
---

# Idle-survival tests for shell-launched apps

`tests-integration/tests/app_lifecycle.rs` (`tetris_survives_idle`, `snake_survives_idle`) was deleted because its cost was pure host wall-clock (a fixed 45 s / 35 s sleep, about 38 % of the x86_64 PR-tier suite; see `plans/integration-baseline-2026-11-07.md`).

The bug class is a triple fault while a shell-launched app runs (idle/wake paths). The replacement must observe guest progress, not host silence: the app (or a purpose-built `itest` app) prints a liveness marker every N ticks of its own timer; the host waits event-driven for K markers with a cap, quits the app and asserts a clean return to `rose>`. A repaint-count variant suits Tetris; the marker scheme covers Snake's static `GAME OVER`. Both arches, release kernels.
