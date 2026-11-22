# Integration-suite speed-up — implementation log

Companion to `plans/integration-speedup-plan.md`. Baseline evidence:
`plans/integration-baseline-2026-11-22.md`.

## Friction log

step 0 | quiet-host gate `pgrep -f 'qemu-system|cargo test'` matched its own inline bash cmdline twice (false BUSY indistinguishable from a real foreign suite); reworked the gate into a script file whose cmdline cannot match | ENV-TRAP | 1 round
step 0 | foreign OpenCode session (worktree `tidy-forest`) held QEMU for ~25 min before the baseline could start; measurements chained behind the wait as the Nov-21 erratum prescribes | TASK | 25 min wall, no rework
step 1 | verify-chain launcher died instantly: outer redirect to `step1/driver.log` opened before the script's own `mkdir` created the dir | ENV-TRAP | 1 round
step 3 | a `read` of `boot_and_exit_status.rs` returned content that no longer matched disk, and looked like a foreign process reverting the file; ~10 min spent hunting a writer that did not exist before `md5sum` + `sed` proved the file matched `HEAD` | ENV-TRAP | ~10 min
step 4 | shared `mem` case first asserted `BootReport`/`PeakAllocKB` markers no code prints (the real test drives `mem` and expects the snapshot table); caught by `cargo test --no-run` before any QEMU time | MISSING-FACT | 1 round
step 4 | group runner used `GROUPS` as its list variable: bash reserves `GROUPS` (numeric group ids), so every `--test` argument was `1000` and the pass reported ten instant failures | ENV-TRAP | 1 round
step 6 | recorded suite walls from a background runner whose tool call had timed out at 120 s while the script kept running; the stamped start/end files gave the true 246 s, so numbers must come from the files, never from a truncated notification | ENV-TRAP | 0 (caught before committing)
step 6 | `sed 's/1aad3c1/0c0f228/'` on the PR body also rewrote the commit-table row naming the Step 6 head; rereading the table before `gh pr create` caught it | ENV-TRAP | 0 (caught before publishing)
merge | main's carrier swap (#197) rewrote the harness paragraph this branch annotated and landed a second AGENTS.md measurement rule in the same sink, duplicating ours; resolved by keeping main's boot story plus our two sentences and folding the two rules into one line with both ledger rows | TASK | 1 round
