# XBX Design System — Platform Mapping

Vue 3 + SCSS conventions, spatial nav wiring, overlay patterns, density overrides.

---

## 1. COMPONENT STRUCTURE TEMPLATE

Every interactive component follows this pattern:

```vue
<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'

interface MyComponentProps {
  id: string
  scopeId: string
  label: string
  disabled?: boolean
}

const props = withDefaults(defineProps<MyComponentProps>(), {
  disabled: false,
})

const emit = defineEmits<{
  (event: 'select'): void
}>()

function handleSelect(): void {
  emit('select')
}
</script>

<template>
  <Focusable
    :id="props.id"
    as="button"
    type="button"
    class="my-component"
    :scope-id="props.scopeId"
    :disabled="props.disabled"
    :aria-label="props.label"
    @click="handleSelect"
  >
    <span class="my-component__label">{{ props.label }}</span>
  </Focusable>
</template>

<style scoped>
.my-component {
  padding: var(--ui-space-lg);
  border: 1px solid var(--ui-border-subtle);
  border-radius: var(--ui-radius-md);
  background: var(--color-surface-1);
  color: var(--color-text-primary);
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast);
}

/* Focus state — REQUIRED */
.my-component[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

/* Cascade focus color to children */
.my-component[data-focused='true'] .my-component__label {
  color: var(--ui-focus-text);
}
</style>
```

---

## 2. SPATIAL NAVIGATION WIRING

### Basic Focusable

```vue
<Focusable
  :id="props.id"           <!-- unique node ID -->
  :scope-id="props.scopeId" <!-- parent FocusScope ID -->
  as="button"
  type="button"
  :disabled="props.disabled"
  :aria-label="props.ariaLabel"
  @click="handleAction"
/>
```

`Focusable` automatically sets `data-focused="true"` when the node receives focus.

### FocusScope for Containers

```vue
<FocusScope
  :id="props.scopeId"
  as="section"
  :active="props.open"
  :default-focus-id="firstItemId"
  :trap="true"
  :restore-focus="true"
>
  <!-- Focusable children -->
</FocusScope>
```

- `active`: whether this scope is currently active (for overlays)
- `default-focus-id`: which node to focus when scope activates
- `trap`: prevent focus from leaving this scope (for modals)
- `restore-focus`: return focus to previous node when scope deactivates

### Explicit Neighbor Hints

```vue
<Focusable
  :id="nodeId"
  :scope-id="scopeId"
  :neighbors="{ up: 'node-above-id', down: 'node-below-id' }"
  :index="{ row: 0, col: 1 }"
/>
```

### Back Button Handling

```vue
<Focusable
  :id="nodeId"
  :scope-id="scopeId"
  :on-back="handleClose"
/>
```

When user presses B button (gamepad) or Escape (keyboard), `handleClose` is called.

---

## 3. OVERLAY PATTERN

All overlays use `Teleport to="body"` to escape ancestor `overflow: hidden`.

```vue
<Teleport to="body">
  <Transition name="my-overlay-transition">
    <div v-if="props.open" class="my-overlay-layer">
      <div class="my-overlay-backdrop" @click="handleClose" />
      <div class="my-overlay-anchor">
        <FocusScope
          :id="props.scopeId"
          as="section"
          class="my-overlay__panel"
          :active="props.open"
          :default-focus-id="firstItemId"
        >
          <!-- content -->
        </FocusScope>
      </div>
    </div>
  </Transition>
</Teleport>
```

**Style pattern:**
```css
.my-overlay-layer {
  position: fixed;
  inset: 0;
  z-index: var(--z-overlay);
}

.my-overlay-backdrop {
  position: absolute;
  inset: 0;
  background: var(--ui-scrim-bg);
}

.my-overlay-anchor {
  position: absolute;
  top: 24px;
  left: 24px;  /* or right: 24px for right-side panels */
  bottom: 24px;
  pointer-events: none;
}

.my-overlay__panel {
  width: min(calc(100vw - 48px), 340px);
  pointer-events: auto;
  background: var(--ui-surface-overlay);
  border: 1px solid var(--ui-border-subtle);
  border-radius: 16px;
  box-shadow: var(--ui-shadow-overlay);
}

/* Animation */
.my-overlay-transition-enter-active,
.my-overlay-transition-leave-active {
  transition: opacity 250ms ease;
}

.my-overlay-transition-enter-active .my-overlay__panel,
.my-overlay-transition-leave-active .my-overlay__panel {
  transition: transform 350ms cubic-bezier(0.2, 0, 0, 1);
}

.my-overlay-transition-enter-from .my-overlay__panel {
  transform: translateX(calc(-100% - 48px)); /* slide from left */
}

.my-overlay-transition-leave-to .my-overlay__panel {
  transform: translateX(calc(-100% - 48px));
}

.my-overlay-transition-enter-from,
.my-overlay-transition-leave-to {
  opacity: 0;
}
```

For right-side panels, use `translateX(calc(100% + 48px))`.

---

## 4. DENSITY OVERRIDES

Density is set via `data-ui-density` on `<html>`. Token overrides live in `_theme-semantic.scss` — never in component files.

For layout that must change at specific densities, use `:global()` inside `<style scoped>`:

```css
/* Default (standard) */
.my-panel {
  width: 340px;
  right: 24px;
}

/* Narrow density — full width */
:global(html[data-ui-density='narrow']) .my-panel {
  width: 100%;
  right: 0;
}

/* Compact density — smaller padding */
:global(html[data-ui-density='compact']) .my-panel {
  padding: 16px;
}
```

Never use media queries for density — use `data-ui-density` attribute selectors.

---

## 5. THEME SWITCHING

Theme is applied via `data-theme` on `<html>`:

```ts
// src/app/theme.ts
export function applyTheme(theme: 'dark' | 'light'): void {
  document.documentElement.dataset.theme = theme
  document.documentElement.style.colorScheme = theme
}
```

CSS responds automatically:
```scss
// _theme-semantic.scss
:root {
  /* dark defaults */
  --ui-page-bg: #0f0f10;
  --ui-page-text: #ffffff;
}

:root[data-theme='light'] {
  /* light overrides */
  --ui-page-bg: #f5f5f5;
  --ui-page-text: #1a1a1a;
}
```

---

## 6. SCSS CONVENTIONS

### Import Order

```scss
@use './theme-semantic';   // imports foundation automatically
@use './token-alias';      // alias bridge
```

Never `@import` — always `@use`.

### Writing Component Styles

```scss
// ✅ Correct — token reference
.my-component {
  padding: var(--ui-space-lg);
  color: var(--color-text-primary);
  border-radius: var(--ui-radius-md);
}

// ❌ Wrong — hardcoded values
.my-component {
  padding: 13px;
  color: #e8e8e8;
  border-radius: 8px;
}
```

### color-mix() Pattern

For transparency variants not covered by tokens:

```css
background: color-mix(in srgb, var(--color-surface-2) 94%, transparent);
border-color: color-mix(in srgb, var(--color-border-subtle) 86%, transparent);
```

---

## 7. PAGE LAYOUT PATTERN

From `src/components/layout/AppShellLayout.vue`:

```vue
<section class="app-shell">
  <FocusScope :id="SPATIAL_NAV_SCOPE_IDS.appShell" :active="!isOverlayOpen">
    <TopNavBar @select="handleNavSelect" />
    <main class="app-shell__content">
      <slot />
    </main>
  </FocusScope>
  
  <!-- Overlays outside main scope -->
  <UserProfileMenu :open="isProfileMenuOpen" @close="closeProfileMenu" />
</section>
```

```css
.app-shell {
  display: flex;
  flex-direction: column;
  width: 100%;
  min-height: 100vh;
  height: 100vh;
  overflow: hidden;
}

.app-shell__content {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: var(--ui-page-inset);
  padding-top: var(--ui-app-shell-content-padding-top);
}
```

---

## 8. ROUTE TRANSITION PATTERN

From `src/styles/base.css`:

```css
.page-fade-enter-active,
.page-fade-leave-active {
  transition: opacity 220ms ease, transform 220ms ease;
}

.page-fade-enter-from {
  opacity: 0;
  transform: scale(0.98);
}

.page-fade-leave-to {
  opacity: 0;
  transform: scale(1.02);
}
```

---

## 9. SCROLL CONTAINER PATTERN

From `HorizontalListRail.vue`:

```css
.viewport {
  overflow-x: auto;
  overflow-y: visible; /* allow scale to overflow */
  scroll-behavior: smooth;
  scroll-padding-inline: var(--shelf-scroll-padding);
  scrollbar-width: none;
  scroll-snap-type: x mandatory;
}

.scroller {
  display: flex;
  gap: var(--shelf-row-gap);
  min-width: max-content;
}

.scroller > * {
  scroll-snap-align: start;
}
```

Use `MutationObserver` to watch `data-focused` and call `scrollIntoView()`.

---

## 10. COMMON PITFALLS

❌ **Don't use `:focus` or `:focus-visible`** — spatial nav sets `data-focused` directly

❌ **Don't use media queries for density** — use `data-ui-density` attribute selectors

❌ **Don't hardcode overlay z-index** — use `var(--z-overlay)` or `var(--z-modal)`

❌ **Don't forget `Teleport to="body"`** for overlays — ancestor `overflow: hidden` will clip them

❌ **Don't forget to cascade focus color to children** — `data-focused` only sets on the root element

✅ **Do read the closest existing component first** — don't invent new patterns

✅ **Do use `:global()` for density overrides** — keeps component styles scoped

✅ **Do use `FocusScope` for overlay containers** — handles focus trap and restoration

✅ **Do use `on-back` prop** — handles B button / Escape key consistently
