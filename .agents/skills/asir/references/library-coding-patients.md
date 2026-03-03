---
name: library-coding-patients
description: Library coding patterns and architecture style for skills and libraries. Use when defining code structure, extension points, and design rules.
---

# Library Coding Patterns

These patterns define the default expectations for library-style code in this repository.

## 1. Architecture Layers and Boundaries
- Core orchestrates lifecycle and flow only.
- Capabilities are injected as modules; core does not depend on concrete implementations.
- Dependencies are one-way from higher to lower layers.

## 2. Functional and Compositional Style
- Prefer pure functions where possible.
- Prefer composition over inheritance.
- Make inputs and outputs explicit.

## 3. Context and Dependency Injection
- Runtime state is managed through context objects.
- Key APIs must require a context to avoid dangling calls.
- Provide composition-friendly APIs that hide internal details.

## 4. Extensibility First
- Design extension points early (hooks, modules, setup).
- Use configuration to drive behavior instead of hardcoding.
- Defaults must be usable; configuration can be incremental.

## 5. Defensive Programming
- Validate public API inputs with clear errors.
- Fail fast on critical path issues.
- Error messages must state what happened, why, and how to fix.

## 6. Type-First Design
- Treat types and interfaces as contracts.
- Add explicit types for complex structures.
- Use naming that reflects purpose, not implementation.

## 7. Naming and Abstraction
- Names should reflect intent, not technical detail.
- Keep naming consistent within the same layer.
- Avoid unnecessary abbreviations.

## 8. Comments and Documentation
- Comments should explain why, not what.
- Document constraints and boundaries explicitly.
- Public APIs and critical flows must be documented.

## 9. High Cohesion, Low Coupling
- Modules should solve one class of problems.
- Collaboration uses interfaces, hooks, or events.
- Minimize visibility; keep internals private when possible.

## 10. Maintainability and Evolution
- Each function does one thing.
- Start simple, add complexity gradually.
- Maintain compatibility or provide migration paths.
