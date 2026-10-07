---
description: Run a development task (a plan file path, or a task description) through the full orchestrator workflow.
agent: orchestrator
---

Run the following development task through the full workflow to completion:

$ARGUMENTS

- If $ARGUMENTS is a path to an existing plan file, have the architect validate it and issue binding resolutions, then run each numbered step through coder -> verifier -> reviewer and commit per step.
- Otherwise treat it as a free-form task: settle plan-changing design questions with the user, run exploration and architecture, then run the plan through coder -> verifier -> reviewer and commit once on approval.
- Stop early only when a stage loops more than twice without progress, or a decision the user must resolve blocks a step.
- When finished (or stopped), report: steps completed with their SHAs, any failures and why, and remaining pending steps.
