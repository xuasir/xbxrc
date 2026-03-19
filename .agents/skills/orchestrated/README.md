# Codex Orchestrator Pack

This pack contains:

- `skills/orchestrated-codex-v2.md`  
  Global skill for main-window orchestration, anti-drift control, routing, confirmation boundaries, and execution defaults.

- `workflows/bugfix-workflow.md`  
  Bug-fix workflow for regression, crash, logic, state, race, and integration issues.

- `workflows/refactor-workflow.md`  
  Refactor workflow for scoped cleanup, structural changes, module extraction, and API-preserving redesign.

- `workflows/feature-workflow.md`  
  Feature delivery workflow for spec-first implementation.

- `workflows/performance-workflow.md`  
  Performance workflow for latency, throughput, memory, and CPU hotspots.

- `agent-prompts/reader-agent.md`  
  Low-cost reader agent prompt.

- `agent-prompts/analysis-agent.md`  
  5.4-mini high-reasoning analysis agent prompt.

- `agent-prompts/coding-agent.md`  
  5.3-codex coding agent prompt.

- `agent-prompts/verifier-agent.md`  
  Validation and acceptance-check agent prompt.

- `agent-prompts/spec-agent.md`  
  Optional spec/contract agent prompt.

- `templates/main-window-response-template.md`  
  Main-window response format.

- `templates/subagent-output-contracts.md`  
  Structured output contracts for all agents.

- `examples/routing-matrix.md`  
  Quick routing matrix.

Recommended use:
1. Load the global skill first.
2. Select one workflow per task type.
3. Use the agent prompts as specialized subagent instructions.
4. Keep the main window clean: goal, plan, state, risks, acceptance only.
