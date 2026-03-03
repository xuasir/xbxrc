---
name: use-spatial-navigation
description: Integrate and debug spatial focus navigation with @spatial-navigation/runtime, @spatial-navigation/react, @spatial-navigation/vue, and @spatial-navigation/dom. Use when implementing keyboard/gamepad directional navigation, configuring FocusScope/Focusable/neighbors/index/tabLevel, handling TAB_NAV (such as LB/RB/LT/RT), or troubleshooting focus, scope, visibility, and input mode issues.
---

# Use Spatial Navigation

## Overview

Deliver practical, minimal, production-ready integration guidance for spatial navigation in React, Vue 3, or framework-agnostic environments.

## Workflow

1. Identify the integration target first:
   - React: use `@spatial-navigation/react`.
   - Vue 3: use `@spatial-navigation/vue`.
   - Framework-agnostic/runtime-level: use `@spatial-navigation/runtime` (optionally with `@spatial-navigation/dom`).
2. Identify the user goal:
   - New setup: provide a minimal working skeleton (Provider + Scope + Focusable).
   - Navigation behavior: define `neighbors` and/or `index`, and assign `tabLevel` where needed.
   - Input behavior: verify keyboard/gamepad sources and `inputMode` updates.
   - Focus issues: verify scope activation, default focus, history restore, and disabled state.
3. Keep output strategy consistent:
   - Provide a minimal solution first, then optional advanced refinements.
   - Depend on public package APIs only.
   - Never depend on repository-internal file paths or private implementation details.
   - Keep node IDs stable, unique, and predictable.

## Implementation Rules

- Ensure every focusable node belongs to an explicit `scopeId`.
- Ensure each active scope has at least one focusable node.
- Prefer explicit `neighbors` links for deterministic movement.
- Use `index: { row, col, order }` for grid/rail fallback behavior.
- For shoulder-button tab switching, assign `tabLevel: 'primary' | 'secondary'` to target nodes.
- In `ConsoleUIProvider`, enable keyboard/gamepad input sources by default unless user requirements say otherwise.
- For overlays/modals, use nested `FocusScope` with `trap` and `restoreFocus`.
- For visibility issues, verify node-to-element registration and `ensureFocusedNodeVisible` wiring.

## Debug Checklist

- Focus does not move:
  - Verify `activeScopeId` matches node `scopeId`.
  - Verify target nodes are not `disabled`.
  - Verify `neighbors` only point to existing IDs.
- `TAB_NAV` does not work:
  - Verify nodes have the expected `tabLevel`.
  - Verify dispatched action shape: `{ type: 'TAB_NAV', level, dir }`.
- Gamepad does not work:
  - Verify gamepad input source is enabled.
  - Verify input polling/listener lifecycle is not prematurely cleaned up.
- Focus style is wrong:
  - React: verify `focusableProps` are applied.
  - Vue: verify `focusableAttrs` map to `tabindex` and `data-focused`.

## References

- Generic setup templates and action patterns: `references/integration-recipes.md`
- Package responsibilities and action model: `references/package-map.md`
