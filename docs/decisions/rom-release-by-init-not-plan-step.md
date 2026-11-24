---
status: decided
date: 2026-11-23
revisit-when: a boot plan needs the ROM's RAM back before init exits
---

## Question
Should releasing the ROM be a boot-plan step (e.g. a `release-rom` line in the
plan) or part of `init`'s own control flow?

## Chosen
`init` releases at the end of its own control flow (commit `9c1c872`): after a
successful `exec` hand-off or at plan exhaustion, `init` calls `RomRelease`
once. Every `rom:` token in the plan is resolved — and its image eagerly copied
into group memory by `LoadElf`, which retains nothing — before `init` returns,
and `exec` ends the plan, so the release is always after the last ROM read. The
`rom:shell` fallback is itself an `exec` loaded before release, so it survives
any plan failure.

## Alternatives
A plan step would require teaching `init` and the plan format a step that only
exists for one resource, and a plan that ends before that step would silently
leak the RAM. A kernel-side "last reader gone" auto-release would need
refcounting across groups that nothing else uses.

## Consequences
No plan-format change; the release cannot be forgotten by a plan author. The
cost: only `init` (the kernel-recorded owner group) can ever release, so a
future reader other than `init` would need the lifecycle reworked. If a plan
ever needs the memory before `init` exits, reopen this record.
