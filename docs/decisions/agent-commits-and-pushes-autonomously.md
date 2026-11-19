---
status: decided
date: 2026-11-19
revisit-when: an autonomous push causes harm outside the working branch
---

## Question
Must the agent ask for approval before every change and commit?

## Chosen
The agent commits per small reviewable step and pushes to its working branch on its own when `cargo xtask check` is green. PR creation stays a manual, user-requested act. The agent stops and asks only for the stop-list in `AGENTS.md`.

## Alternatives
The earlier "work incrementally, ask before each change" rule.

## Consequences
Instruction files (`AGENTS.md`, `GEMINI.md`, `.opencode/`) must not ask for approval before changes.
