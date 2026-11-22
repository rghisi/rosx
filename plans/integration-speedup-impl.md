# Integration-suite speed-up — implementation log

Companion to `plans/integration-speedup-plan.md`. Baseline evidence:
`plans/integration-baseline-2026-11-22.md`.

## Friction log

step 0 | quiet-host gate `pgrep -f 'qemu-system|cargo test'` matched its own inline bash cmdline twice (false BUSY indistinguishable from a real foreign suite); reworked the gate into a script file whose cmdline cannot match | ENV-TRAP | 1 round
step 0 | foreign OpenCode session (worktree `tidy-forest`) held QEMU for ~25 min before the baseline could start; measurements chained behind the wait as the Nov-21 erratum prescribes | TASK | 25 min wall, no rework
step 1 | verify-chain launcher died instantly: outer redirect to `step1/driver.log` opened before the script's own `mkdir` created the dir | ENV-TRAP | 1 round
