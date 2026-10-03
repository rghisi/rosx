# Multi-Agent Workflow Improvements Plan

## Context
Derived from the Unified Build `xtask` execution on `project-level-build`. The multi-agent loop Explorer → Architect → Coder → Reviewer → Committee succeeded in delivering the feature but exposed gaps in plan adherence, review gating, and commit reliability.

## Goals
- Enforce locked decisions and acceptance criteria across agents
- Make Reviewer a blocking, structured gate
- Harden Committee commit step
- Reduce manual fix iterations
- Improve auditability

## Proposed Changes

### 1. Plan-Lock Checklist
**Owner:** Architect
**Change:** Architect outputs a machine-readable checklist `plan.lock.json` containing:
- Locked decisions with IDs
- Acceptance criteria with verifiable commands
- Interfaces/contracts for Coder

**Workflow impact:** Orchestrator verifies Coder attests to each item before starting.

### 2. Structured Reviewer Gate
**Owner:** Reviewer
**Change:** Reviewer must return:
- APPROVED / NOT APPROVED
- Structured fix list: `file`, `line`, `rule`, `severity`, `required change`
- Acceptance criteria coverage report

**Workflow impact:** Orchestrator auto-routes fix list back to Coder as a single task. No manual triage.

### 3. Resilient Committee
**Owner:** Committee
**Change:** Committee produces commit message and performs `git add/commit/push` atomically.
- Retry with exponential backoff on API/connect failures
- Fallback to local commit with clear warning if push fails
- Enforce commit message convention: no prefix, plain sentence

### 4. Coder Output Contract
**Owner:** Coder
**Change:** Coder must return:
- Files changed/created/deleted list
- Verification commands run + exit codes + key artifacts
- Static checks: no comments in Rust, no `unsafe` without justification

**Workflow impact:** Reviewer can validate without re-running everything.

### 5. Explorer → Architect Delta
**Owner:** Orchestrator
**Change:** If Explorer finds discrepancies vs plan, Architect must emit `plan.delta.md` with changes and re-approval step before coding starts.

### 6. Transient Breakage Communication
**Owner:** Architect
**Change:** Explicit `known_transient_breakage` section in plan.
Orchestrator surfaces it to user at step start.

### 7. Docs Sync Gate
**Owner:** Step 5 of plans
**Change:** Docs updates `AGENTS.md`/`README.md` must pass a diff check against plan-required sections. Separate sub-task.

### 8. xtask Self-Test
**Owner:** Coder
**Change:** Add lightweight unit tests for `xtask` argument parsing and workspace-root pinning. Run as part of `cargo test --workspace`.

## Implementation Steps
1. Create `plans/multi-agent-workflow.md` with these changes
2. Update Orchestrator prompts to enforce checklist attestation
3. Update Reviewer prompt to require structured fix list
4. Update Committee prompt with retry/fallback logic
5. Add Coder output template to AGENTS.md
6. Pilot on next plan

## Success Criteria
- Zero coder deviations from locked decisions on next plan
- Reviewer NOT APPROVED triggers automated fix loop, not manual
- Committee commit succeeds without manual fallback
- Full audit trail from Explorer to Commit in one report

## Risks
- Over-formalization may slow small changes
- Mitigation: keep checklist lightweight for <3 step tasks
