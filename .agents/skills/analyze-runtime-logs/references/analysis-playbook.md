# Runtime Log Analysis Playbook

## Use This Playbook For

- Startup or provisioning failures
- Streaming session regressions
- RTC or media pipeline stalls
- Performance review from runtime traces
- Cross-trace comparison after a code change

## Analysis Procedure

### 1. Define The Question

Classify the task before reading everything:

- incident triage
- regression verification
- performance evaluation
- comparison between two traces

This changes what “important” means.

### 2. Establish The Skeleton Timeline

Collect:

- first and last `tsMs`
- duration
- active `sessionId`
- first major `state` rows
- terminal failure or recovery rows

Do not start from free-form `log` spam.

### 3. Find The First Abnormal Signal

Search for the earliest row that indicates drift:

- timeout
- unavailable
- dropped
- stall
- reconnect
- recovery escalation
- missing capability or null capability snapshot
- no ingress / no media / no ICE progress

The first abnormal signal is usually more useful than the last emitted error.

### 4. Correlate Across Layers

Map the trace across these boundaries:

- UI or command trigger
- Tauri service orchestration
- data or preflight snapshots
- `xbxengine` runtime observations
- transport / media / recovery consequences

Do not infer an engine fault when the trace already shows upstream provisioning or capability failure.

### 5. Separate Failure Modes

Keep these cases distinct:

- service-side provisioning never becomes ready
- capability or remote-play preflight is missing required fields
- transport is connected but media never flows
- media flows but recovery cannot obtain a keyframe
- media ingress is present but local pipeline is backpressured
- performance is degraded without a terminal failure

### 6. Evaluate Performance Explicitly

For performance analysis, always state:

- measured time window
- startup or recovery phase boundaries
- obvious long gaps between milestones
- repeated retries, reconnects, or resets
- sustained low-throughput / no-throughput windows
- whether evidence points to network, provisioning, control-plane, or local pipeline pressure

## Output Format

Use this template unless the user asks for something else:

### Scope

- file(s)
- question answered

### Timeline

- earliest relevant anchors with `seq` / `tsMs`
- first abnormal signal
- terminal symptom or recovery point

### Findings

- high-confidence finding
- secondary observations
- competing hypotheses if confidence is limited

### Evidence

- rows, payload fields, and transitions that support the conclusion

### Gaps

- what the trace does not prove
- which instrumentation is still missing

### Next Step

- concrete code path to inspect
- extra trace to capture
- targeted experiment to run

## Project-Specific Hints

- Prior analyses are recorded in `docs/project-task.md` and archived task files. Reuse prior terminology when a trace resembles an earlier incident.
- `remoteManagementEnabled`, `consoleStreamingEnabled`, and address-count snapshots are often decisive for home streaming preflight analysis.
- For `xbxengine` traces, watch for transport progress, ingress activity, recovery state, keyframe flow, and backlog or capacity warnings before blaming rendering.
- When performance degrades without a hard failure, compare `snapshot` rows and recovery decisions before focusing on individual debug lines.
