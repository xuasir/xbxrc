# Workflow: refactor-workflow

Use this workflow for:
- module cleanup
- internal API reshaping
- dependency inversion
- extraction
- decomposition
- reducing complexity
- preparing for future work

## Objective
Improve structure without breaking behavior, while keeping scope explicit and reversible.

## Phase 0 - Declare refactor intent
Main window must define:
- target area
- reason for refactor
- expected benefit
- behavior that must stay identical
- forbidden expansion areas

## Phase 1 - Read structure
Dispatch Reader Agent to gather:
- current module responsibilities
- key symbols
- call boundaries
- dependency hotspots
- likely seams for extraction

## Phase 2 - Design the minimal refactor
Dispatch Analysis Agent.

Analysis output must include:
- structural problem
- minimal viable refactor
- alternative approach if simpler
- migration order
- breakage risk
- preserved invariants

Reject solutions that are elegant but oversized.

## Phase 3 - Lock refactor plan
The main window must publish:
- intended end state
- step order
- allowed files
- whether public APIs must remain unchanged
- verification plan

## Phase 4 - Implement in small patches
Dispatch Coding Agent in one or more atomic steps.

Preferred patch order:
1. preparatory internal changes
2. move/extract logic
3. remove dead paths
4. align tests
5. optional cleanup only if safe

Each patch should be independently explainable and reversible.

## Phase 5 - Verify behavior preservation
Dispatch Verifier Agent.

Verify:
- build
- tests
- no acceptance regressions
- no unexpected public API break
- no behavior drift

## Phase 6 - Close
Report:
- what improved
- what stayed intentionally unchanged
- any deferred cleanup
- risk and rollback notes

## Guardrails
- no broad renaming for style only
- no new abstraction unless it pays for itself immediately
- no unrelated cleanup
- no “while I’m here” edits
