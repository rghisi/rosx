---
name: Retro
description: Retrospective for RosX work. Use at the end of every plan step, as part of the step's commit report, and when the user asks for a retro review.
---

# Retro

Two modes. Never edit `AGENTS.md`, this skill, or any rule file without approval.

Scope: single-agent work. The multi-agent workflow mode retrospects through the `refiner` agent and `/refine`; this skill does not restate those rules.

## Mode 1: step end

1. Run `bash .opencode/skills/retro/scripts/signals.sh` and read the output before judging the step. For a branch cut from something other than `main`, pass the base ref as the first argument or set `RETRO_BASE`.
2. Append each friction event to the `## Friction log` section of the plan's `-impl` file, one line:
   `<step> | <event> | <CAUSE-CLASS> | <cost>`
   Cause classes and routing: `references/cause-classes.md`. Entry format: `references/friction-log.md`.
3. Report to the user, as part of the step's commit report:
   - `Friction:` none, or 1-3 lines with cause class and cost.
   - `Lesson:` none, or one proposed change and the sink it lands in.
4. For a lesson that is a process rule, run `bash .opencode/skills/retro/scripts/recurrence.sh <CAUSE-CLASS> [keyword]`.
   A process rule needs the same friction in two plans. A factual correction needs one.
5. Propose the change as a diff with its evidence and an expected effect. Apply it only after approval, in its own commit, separate from code, and record it in `docs/rules-ledger.md`; a lesson that is a decision also gets a `docs/decisions/` record (`references/ledger.md`).

First-run gate failures are signals too: record every acceptance command that failed on its first execution.

## Mode 2: review

Run when the user asks for a retro review.

1. Read the last N `-impl` friction logs (default 5, or as the user says) and run `cargo xtask decisions` for the open records.
2. Count cause classes, list recurring events, and run `recurrence.sh` for each recurring cause.
3. For every rule in `docs/rules-ledger.md`, check its expected effect against the logs. Format: `references/ledger.md`.
4. Propose additions, automations and removals. Removing a rule that `cargo xtask check` now enforces is part of the job.
5. Report as a numbered list. Apply nothing without approval.

## Routing

Prefer the strongest sink: a gate (a `cargo xtask check` stage, a unit test, a CI step), then a one-line fact in the owning `docs/architecture/*.md` — with an `AGENTS.md` invariants-index line only when the fact is an invariant — then a judgement rule in `AGENTS.md`, then a `docs/todo/` file with a trigger.
