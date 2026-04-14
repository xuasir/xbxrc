# XBX Design System — Components

Patterns extracted from real implementations in `src/components/`. Always read the source file before creating a variant.

---

## 1. CARDS

### GameCard (`src/components/common/GameCard.vue`)

Square card with image fill + title overlay at bottom.

**Key pattern:**
- Size: `var(--ui-game-card-size)` (176px → 152px → 140px by density)
- Border: `1px solid color-mix(in srgb, var(--color-border-subtle) 86%, ...)`
- Hover: `filter: brightness(1.03)`
- Focus: `background: var(--color-focus-bg)` + `box-shadow: var(--shadow-xbox-focus)` + `transform: scale(1.02)`

### ConsoleStatusCard (`src/components/common/ConsoleStatusCard.vue`)

Vertical card: image top, text stack below.

**Key pattern:**
- Width: `clamp(var(--ui-console-card-min-width), 24vw, var(--ui-console-card-width))`
- Focus: `background: var(--color-focus-bg-strong)` + cascade color to all children
- Shadow: `box-shadow: var(--ui-shadow-floating)` at rest, `var(--shadow-xbox-focus)` on focus

---

## 2. SETTINGS ROWS

### SettingToggleRow (`src/components/settings/SettingToggleRow.vue`)

Full-width button with label left, toggle switch right.

**Key pattern:**
- Border: `2px solid transparent` at rest, `border-color: var(--color-focus-ring)` on focus
- Track: `var(--ui-settings-toggle-track-width)` × `var(--ui-settings-toggle-track-height)`
- Active track: `background: var(--brand-primary)`, `border-color: transparent`
- Thumb: `transform: translate(var(--ui-settings-toggle-thumb-offset), -50%)` when active

### SettingInlineSingleSelect (`src/components/settings/SettingInlineSingleSelect.vue`)

Segmented control below a setting row.

**Key pattern:**
- Margin: `margin-top: -8px` to eat the section gap
- Rail: `display: flex`, `gap: 6px`, `padding: 6px`
- Selected segment: `background: var(--brand-primary)` + green glow shadow
- Focus: `border-color: var(--color-focus-ring)` + `transform: scale(1.02)`

---

## 3. OVERLAYS

### StreamActionSheet (`src/components/stream/StreamActionSheet.vue`)

Fixed layer, slides from left.

**Key pattern:**
- Layer: `position: fixed; inset: 0; z-index: var(--z-overlay)`
- Backdrop: `background: var(--ui-scrim-bg)`
- Panel: `width: min(calc(100vw - 48px), 340px)`, `border-radius: 16px`
- Animation: panel `transform: translateX(calc(-100% - 48px))` for enter/leave
- Eyebrow: `font-size: 13px`, `font-weight: 700`, `letter-spacing: 0.12em`, `text-transform: uppercase`

### UserProfileMenu (`src/components/navigation/UserProfileMenu.vue`)

Slides from right, similar structure.

**Key pattern:**
- Panel slides from right: `transform: translateX(calc(100% + 48px))`
- At narrow density: `:global(html[data-ui-density='narrow']) .user-menu-anchor { left: 24px; }`

---

## 4. NAVIGATION

### TopNavBar (`src/components/navigation/TopNavBar.vue`)

Horizontal flex: left | center | right groups.

**Key pattern:**
- Icon buttons: circular, `--ui-size-control-lg`, `border-radius: 999px`
- Active state: `background: var(--color-state-selected)`

### SpatialNavTabs (`src/components/navigation/SpatialNavTabs.vue`)

Horizontal button list with animated underline.

**Key pattern:**
- Underline: `::after` pseudo-element, `height: var(--ui-tabs-underline-height)`
- Active: `opacity: 1` on `::after`, `color: var(--color-text-primary)`
- Focus: `background: var(--color-focus-bg)` + `box-shadow: var(--shadow-xbox-focus)`

---

## 5. LISTS / RAILS

### HorizontalListRail (`src/components/common/HorizontalListRail.vue`)

Horizontal scroll container with auto-scroll to focused item.

**Key pattern:**
- Viewport: `overflow-x: auto`, `overflow-y: visible`, `scroll-snap-type: x mandatory`
- Scroller: `display: flex`, `gap: var(--shelf-row-gap)`, `min-width: max-content`
- Children: `scroll-snap-align: start`
- Use `MutationObserver` to watch `data-focused` and call `scrollIntoView({ behavior: 'smooth', inline: 'nearest' })`

---

## 6. LOADING / EMPTY STATES

### BrandedLoading (`src/components/common/BrandedLoading.vue`)

Animated ring + logo with optional label.

**Key pattern:**
- Sizes: `xs` / `sm` / `md` / `lg` / `xl` via CSS custom properties
- Ring: `conic-gradient` with `mask-composite: exclude`
- Aura: `radial-gradient` with `color-mix()` for brand green glow
- Animation: `ring-rotate 1.2s cubic-bezier(0.4, 0, 0.2, 1) infinite`

---

## 7. MODALS

### SettingModalShell (`src/components/settings/SettingModalShell.vue`)

Centered modal with backdrop, uses `Teleport to="body"`.

**Key pattern:**
- Layer: `position: fixed; inset: 0; z-index: var(--z-modal)`
- Panel: `padding: var(--ui-settings-modal-panel-padding)`, `gap: var(--ui-space-lg)`
- Eyebrow: `font-size: 12px`, `color: var(--brand-primary)`
- Animation: layer opacity `300ms`, panel `transform: scale(0.95)` → `scale(1)` at `400ms`

---

## COMMON PATTERNS

### Focus State (Universal)

```css
.component[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02); /* optional */
  z-index: 10;
}

/* Cascade to children */
.component[data-focused='true'] .component__label {
  color: var(--ui-focus-text);
}
```

### Transition (Universal)

```css
transition:
  border-color var(--ui-motion-fast),
  background-color var(--ui-motion-fast),
  box-shadow var(--ui-motion-fast),
  transform var(--ui-motion-fast);
```

### Overlay Animation (Universal)

```css
/* Layer */
.overlay-transition-enter-active,
.overlay-transition-leave-active {
  transition: opacity 250ms ease;
}

/* Panel */
.overlay-transition-enter-active .panel,
.overlay-transition-leave-active .panel {
  transition: transform 350ms cubic-bezier(0.2, 0, 0, 1);
}

.overlay-transition-enter-from .panel {
  transform: translateX(calc(-100% - 48px)); /* or translateY, scale, etc. */
}
```
