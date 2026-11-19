---
priority: low
date: 2026-10-11
trigger: a third consumer appears or the two copies need a shared behavior change
---

# benchkit: share the benchmark window mechanism

`apps/bench/src/windows.rs` (calibrate, warm-up, best-of-N measurement windows, the `bench.<phase>.best` line convention) is duplicated in spirit in `apps/fsbench/src/windows.rs`, whose constants are retuned for slow filesystem operations (calibration floors at 1 iteration).

Extract a small `no_std` `benchkit` crate (workspace path dependency). `bench` keeps `ns_per_op`; `fsbench` will likely want `bytes_per_sec`; that divergence signals the shared shape.
