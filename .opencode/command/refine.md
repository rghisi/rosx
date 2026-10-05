---
description: Refine the agent specifications using this session's workflow run.
---

Assemble the run retrospective of the workflow run in this session: the task and final outcome, plan revisions and their reasons, review iterations and required fixes, coder deviations, loop events, and any lossy handoffs. Delegate the full retrospective to the `refiner` subagent. Report the refiner's assessment to me and, if it changed any files under `.opencode/agent/` or `.opencode/command/`, hand the assessment and changed spec files to the `committee` subagent to produce a `chore(agents): ...` commit message with the assessment in the body, then commit only those specification files. If this session contains no completed workflow run, say so and change nothing.
