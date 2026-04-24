---
name: decision-trace
description: Trace why a design, workflow, field, constraint, or architectural direction exists in this repository. Use when the user needs the decision chain behind current behavior, wants to know what assumptions still hold, or needs to separate current policy from older discarded ideas.
---

# Decision Trace

Use this skill to answer "why is it like this now?" with repository evidence.

## Search Scope

Prioritize:

- recent RFCs
- matching reports
- relevant task lines in `docs/project-task.md`
- earlier ISU notes only when they explain the origin of the decision

## Workflow

1. Identify the target decision, field, subsystem, or constraint.
2. Search for the first strong signal of that decision in RFCs and reports.
3. Reconstruct the sequence of refinements, reversals, or closures.
4. Separate current-valid constraints from outdated assumptions.
5. State the highest-confidence present-day conclusion.

## Reporting Contract

Return:

1. Current answer
2. Decision chain
3. Supporting documents
4. Constraints that still apply
5. Assumptions that no longer apply

## Guardrails

- Do not treat old brainstorming as the final decision when later RFCs or reports supersede it.
- Do not collapse rationale and implementation detail into one blob.
- Surface uncertainty when the trail is incomplete.
