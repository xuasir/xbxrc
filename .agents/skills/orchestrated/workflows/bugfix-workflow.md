# Workflow: bugfix-workflow

Use this workflow for:
- regressions
- crashes
- incorrect behavior
- state bugs
- race conditions
- lifecycle bugs
- integration breakages

## Objective
Identify the narrowest root cause and apply the smallest safe fix that satisfies acceptance criteria.

## Phase 0 - Lock the problem
Capture:
- observed symptom
- expected behavior
- actual behavior
- affected surface
- severity
- reproducibility
- acceptance checks

Main-window output:
- one-line bug statement
- non-goals
- risk level

## Phase 1 - Read narrowly
Dispatch Reader Agent to:
- locate likely modules
- identify entry points
- extract relevant symbols
- list 3-10 files max unless evidence requires more

Reader output must include:
- probable bug site
- supporting files
- unknowns

## Phase 2 - Diagnose
Dispatch Analysis Agent with only relevant files and bug statement.

Analysis Agent must produce:
- most likely root cause
- evidence
- competing hypotheses
- minimal recommended fix
- risk of regressions
- patch scope recommendation

If root cause is still uncertain:
- authorize one more narrow read pass
- avoid broad repo exploration

## Phase 3 - Define patch scope
Before coding, explicitly set:
- files allowed to change
- tests required
- behavior that must remain unchanged
- whether API changes are forbidden

## Phase 4 - Implement
Dispatch Coding Agent.

Implementation priorities:
1. correctness
2. minimal scope
3. compatibility
4. clarity

Do not allow opportunistic refactors unless they are required for correctness.

## Phase 5 - Verify
Dispatch Verifier Agent.

Verification ladder:
- build
- lint if relevant
- direct tests for the bug
- nearby regression checks
- acceptance criteria review

## Phase 6 - Close
Report:
- root cause
- changed files
- tests added/updated
- residual risk
- follow-up if needed

## Fast Path
If the bug is obviously localized and low risk:
Reader -> Coding -> Verifier

## Escalation Rules
Escalate back to Analysis Agent if:
- first implementation fails twice
- the bug crosses module boundaries unexpectedly
- proposed fix requires API change
- evidence contradicts the diagnosis
