# Workflow: performance-workflow

Use this workflow for:
- latency reduction
- throughput improvement
- memory reduction
- hot path optimization
- frame scheduling / queue control / backpressure work

## Objective
Make the measured or clearly-defined bottleneck better without degrading correctness or expanding scope unnecessarily.

## Phase 0 - Define the performance target
Capture:
- metric
- current symptom
- target
- workload shape
- non-negotiable correctness constraints
- acceptable tradeoffs

## Phase 1 - Read hot-path only
Dispatch Reader Agent to identify:
- hot path modules
- queue/state boundaries
- scheduler/control points
- measurement hooks
- files likely involved

## Phase 2 - Analyze the bottleneck
Dispatch Analysis Agent.

Analysis output must include:
- bottleneck hypothesis
- evidence
- dominant control points
- minimal intervention candidates
- tradeoffs
- regression risks

Require explicit separation of:
- correctness issues
- scheduling policy issues
- pure performance opportunities

## Phase 3 - Choose minimal intervention
Main window selects one:
- instrumentation only
- local algorithm change
- queue policy change
- scheduling policy change
- larger redesign

Prefer the smallest move that can prove or improve the target.

## Phase 4 - Implement
Dispatch Coding Agent.

Guidelines:
- isolate fast-path changes
- avoid broad rewrites
- preserve external behavior unless explicitly approved
- add measurements/tests where possible

## Phase 5 - Verify
Dispatch Verifier Agent.

Verify:
- build/tests
- target metric direction
- no obvious behavior regression
- no unacceptable CPU/memory trade introduced elsewhere

## Phase 6 - Close
Report:
- what changed
- expected performance impact
- what remains unproven
- next measurement step
