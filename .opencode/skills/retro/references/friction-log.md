# Friction log format

Section `## Friction log` in the plan's `plans/<name>-impl.md`, created with the plan and appended to as events happen. A plan that is a single file carries the section there instead.

    <step> | <event> | <CAUSE-CLASS> | <cost>

- step: plan step number or name
- event: one sentence, concrete (command, file, symptom)
- CAUSE-CLASS: exactly one class from cause-classes.md
- cost: rounds of rework or a short time estimate; 0 when negligible

Example:

    step 2 | built rosx-x86_32 from repo root, exit 101 | ENV-TRAP | 1 round

An entry with fewer than three `|`-separated columns is invisible to `scripts/recurrence.sh`.
