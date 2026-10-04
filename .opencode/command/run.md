---
description: Run a development task (a plan file path, or a task description) through the worker -> verifier -> reviewer -> committer pipeline.
agent: orchestrator
---

Run the following development task through the full pipeline to completion:

$ARGUMENTS

- If $ARGUMENTS is a path to an existing plan file, treat that file as the plan.
- Otherwise treat it as a free-form task: break it into a numbered step list (each step: goal, files to touch, constraints, verification command, commit message), present the step list to the user and wait for confirmation, then run the pipeline autonomously.
- Run every step through worker -> verifier -> reviewer -> committer, updating the per-task PROGRESS file as you go.
- Stop early only if a step fails after its retry budget, or a decision the user must resolve blocks a step.
- When finished (or stopped), report: steps completed with their SHAs, any failures and why, and remaining pending steps.
