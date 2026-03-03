# Package Map

## Public Packages

- `@spatial-navigation/runtime`
  - Role: core runtime state machine, action dispatch, scope/node lifecycle, plugin pipeline.
- `@spatial-navigation/dom`
  - Role: DOM bridge for measurement/focus, visibility scrolling, and layout invalidation hooks.
- `@spatial-navigation/react`
  - Role: React provider, scope components, focusable components, and hooks.
- `@spatial-navigation/vue`
  - Role: Vue provider, scope components, focusable components, and composables.

## Action Model

- `NAV`: directional navigation (`up/down/left/right`).
- `CONFIRM`: confirm/accept action.
- `BACK`: back action.
- `MENU`: menu action.
- `TAB_NAV`: tab-level switching (`primary/secondary` + `prev/next`).

## Core Data Contracts

- `ScopeDef`
  - `id`: unique scope identifier.
  - `parentId`: parent scope relationship for nested navigation.
  - `trap`: constrain focus within the scope.
  - `defaultFocusId`: preferred focus target when scope becomes active.
  - `restoreFocus`: recover previous focus history for the scope.
- `NodeDef`
  - `id/scopeId`: node identity and scope ownership.
  - `disabled`: non-focusable marker.
  - `neighbors`: explicit directional edges.
  - `index`: optional grid/rail ordering hints.
  - `tabLevel`: target group for `TAB_NAV`.

## Generic Reading Order

1. Start with public package exports (React or Vue).
2. Add provider/scope/focusable wiring.
3. Define navigation graph with `neighbors/index/tabLevel`.
4. Validate focus transitions with runtime actions.
