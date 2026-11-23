---
priority: low
date: 2026-11-22
trigger: a port or experiment wants a first program other than `init`
---

# Make the init program name a kernel parameter

The kernel hardcodes the name `init` when it picks the first program out of the initrd ROM (`kernel::rom::program()` called from both arch mains). That is the right default and the panics name it, but the name is now policy baked into two call sites. If a bring-up experiment wants a different first program (a diagnostic init, a per-arch init), lift the name into the arch boot path — a parameter to the shared boot-from-initrd helper, not a new boot-plan concept — and keep `init` as the shipped value on both ports.
