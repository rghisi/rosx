# Cause classes

| Class | Meaning | Preferred fix |
|---|---|---|
| MISSING-FACT | Context lacked or misstated a true fact about the repo | One factual line in the owning `docs/architecture/*.md`, plus an `AGENTS.md` invariants-index line when it is an invariant |
| STALE-DOC | A doc claimed something the change invalidated | Fix the line; add a drift check when it recurs |
| UNGUARDED | A mistake a check could have caught but none did | A `cargo xtask check` stage, a unit test or a CI step |
| PLAN-DEFECT | The plan was ambiguous, wrong or out of order | Adjust the plan-writing rules |
| ENV-TRAP | Toolchain, cwd, harness or flag footgun | Fix the tool; else one line under `AGENTS.md` Toolchain notes |
| TASK | Genuinely hard, nothing to fix | None |

Ideas that are not failures become a `docs/todo/` file with a trigger.
