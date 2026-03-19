# Workflow: feature-workflow

Use this workflow for:
- new capabilities
- new endpoints
- new UI behavior
- new flags/options
- additive flows

## Objective
Deliver the smallest useful feature increment that satisfies an explicit contract.

## Phase 0 - Define the contract
Use the main window or Spec Agent to define:
- user-facing goal
- inputs
- outputs
- invariants
- non-goals
- acceptance criteria

## Phase 1 - Read existing seams
Dispatch Reader Agent to identify:
- insertion points
- affected modules
- existing patterns worth reusing
- tests that should mirror the new behavior

## Phase 2 - Design minimally
Dispatch Analysis Agent only if:
- feature crosses modules
- the design is unclear
- performance/state concerns exist
- compatibility matters

Analysis output:
- recommended design
- why it is minimal
- files likely to change
- risk areas
- rollout plan if needed

## Phase 3 - Lock patch scope
Define:
- allowed files
- expected behavior
- forbidden refactors
- tests to add

## Phase 4 - Implement
Dispatch Coding Agent.

Preferred order:
1. data model / config support
2. core behavior
3. integration glue
4. tests
5. optional docs/comments if asked

## Phase 5 - Verify
Dispatch Verifier Agent.

Check:
- build
- direct tests
- nearby regressions
- acceptance criteria
- non-goals preserved

## Phase 6 - Close
Report:
- feature delivered
- test status
- remaining edge cases
- next increment if any
