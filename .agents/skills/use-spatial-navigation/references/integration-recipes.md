# Integration Recipes

## React Quick Start

```tsx
import { ConsoleUIProvider, Focusable, FocusScope } from '@spatial-navigation/react'

export default function Page() {
  return (
    <ConsoleUIProvider inputSources={{ keyboard: true, gamepad: true }}>
      <FocusScope id="home" defaultFocusId="nav-home">
        <Focusable
          id="nav-home"
          as="button"
          neighbors={{ right: 'nav-store', down: 'card-1' }}
        >
          Home
        </Focusable>
        <Focusable
          id="nav-store"
          as="button"
          neighbors={{ left: 'nav-home', down: 'card-2' }}
        >
          Store
        </Focusable>
      </FocusScope>
    </ConsoleUIProvider>
  )
}
```

Implementation notes:
- Ensure `defaultFocusId` always points to an existing focusable node.
- Prefer explicit `neighbors` in complex layouts to avoid ambiguous movement.

## Vue Quick Start

```vue
<script setup lang="ts">
import { ConsoleUIProvider, Focusable, FocusScope } from '@spatial-navigation/vue'
</script>

<template>
  <ConsoleUIProvider :input-sources="{ keyboard: true, gamepad: true }">
    <FocusScope id="home" default-focus-id="nav-home">
      <Focusable id="nav-home" as="button" :neighbors="{ right: 'nav-store' }">
        Home
      </Focusable>
      <Focusable id="nav-store" as="button" :neighbors="{ left: 'nav-home' }">
        Store
      </Focusable>
    </FocusScope>
  </ConsoleUIProvider>
</template>
```

Implementation notes:
- Use kebab-case props in templates, such as `default-focus-id` and `input-sources`.

## Runtime Quick Start (Framework Agnostic)

```ts
import { createRuntime } from '@spatial-navigation/runtime'

const runtime = createRuntime({ rootScopeId: 'root' })

runtime.registerScope({ id: 'root', defaultFocusId: 'a', restoreFocus: true })
runtime.registerNode({ id: 'a', scopeId: 'root', neighbors: { right: 'b' } })
runtime.registerNode({ id: 'b', scopeId: 'root', neighbors: { left: 'a' } })

runtime.setActiveScope('root')
runtime.dispatch({ type: 'NAV', dir: 'right' })
```

Implementation notes:
- Movement happens only among enabled nodes in the active scope.
- `TAB_NAV` depends on `tabLevel` to switch primary/secondary tab groups.
- For geometry-based navigation, use `createSpatialNavigatorPlugin` with a DOM bridge.

## Tab Navigation Pattern

```ts
runtime.dispatch({ type: 'TAB_NAV', level: 'primary', dir: 'next' })
runtime.dispatch({ type: 'TAB_NAV', level: 'secondary', dir: 'prev' })
```

Implementation notes:
- For tab switching, assign `tabLevel` and stable `index.order` values.
- Common input mapping: `LB/RB -> primary`, `LT/RT -> secondary`.
