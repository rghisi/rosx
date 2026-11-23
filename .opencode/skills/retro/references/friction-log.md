# Friction log format

Section `## Friction log` in the plan's `plans/<name>-impl.md`, created with the plan and appended to as events happen. A plan that is a single file carries the section there instead.

    <step> | <event> | <CAUSE-CLASS> | <cost>

- step: plan step number or name
- event: one sentence, concrete (command, file, symptom)
- CAUSE-CLASS: exactly one class from cause-classes.md
- cost: rounds of rework or a short time estimate; 0 when negligible

Example:

    step 2 | built rosx-x86_32 from repo root, exit 101 | ENV-TRAP | 1 round

`scripts/recurrence.sh` reads the cause class from the second-to-last `|`-separated column and the cost from the last, so an event sentence may quote pipes freely; an entry with fewer than four columns is invisible to it.
