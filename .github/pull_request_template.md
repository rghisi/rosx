## What this is

One or two sentences: the plan, issue or goal this PR carries out, and what it changes at the highest level. Name the plan file under `plans/` when there is one.

## Commits

One line per commit or milestone, in order, saying what each one does and why it stands alone. A table of commit hash and step works well for a series; a single commit needs no table.

## Semantics

The behaviour, contracts and decisions a reviewer must know that the diff does not make obvious: what is now guaranteed, what changed on a boundary (ABI, wire, gate rules), and which decisions or deferrals were recorded in `docs/decisions/` or `docs/todo/`.

## Where reality differed from the plan

Anything the work found that contradicted the plan or the docs, and what was done about it. Omit the section when nothing differed.

## Verification

What was run and what it showed: `cargo xtask check` on the final head, the tests added and their counts, QEMU or integration suites per architecture, and any deliberate failure used to prove a gate. State exact commands and results, not "tests pass".

## Not verified

What was not run or cannot be shown here (hardware, a suite that needs tools the authoring environment lacks, a path covered only by compile checks), stated plainly so a reviewer knows where to look.

## Out of scope

What this PR deliberately leaves unbuilt, and where the follow-up is tracked.
