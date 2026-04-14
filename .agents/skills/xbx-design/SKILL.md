---
name: xbx-design
description: This skill should be used when the user explicitly says "xbx style", "xbx design", "/xbx-design", or directly asks to use/apply the XBX design system. NEVER trigger automatically for generic UI or design tasks.
version: 2.1.0
allowed-tools: [ReadFile, ApplyPatch, Glob, rg]
---

# XBX UI Design System

A gamepad-first desktop UI toolkit for the Xbox Remote Client (xbxrc). Every pattern here is extracted from the actual codebase — not invented. When in doubt, read the real component first.

**Stack:** Vue 3 + TypeScript + SCSS + CSS custom properties. No Tailwind. No utility classes.

**Source of truth:**
- Tokens: `src/styles/tokens.css`, `src/styles/_theme-foundation.scss`, `src/styles/_theme-semantic.scss`
- Components: `src/components/`
- Pages: `src/pages/`

---

---

## 1. CORE PRINCIPLES

**Gamepad is primary input.** Every interactive element must be a `Focusable` node. Mouse/keyboard are secondary.

**Token-first.** Prefer CSS custom properties for spacing, color, size, radius, motion, and hierarchy decisions. Avoid raw hex and arbitrary px values in `<style scoped>`. Limited structural constants (e.g. 1–2px hairlines, precise effect parameters already used in nearby components) are allowed when they match existing patterns. When a semantic token is missing, add it to `_theme-semantic.scss` instead of inventing one-off values.

**Density is automatic.** Four levels (`comfortable` / `standard` / `compact` / `narrow`) are driven by `data-ui-density` on `<html>`. Token overrides in `_theme-semantic.scss` handle the rest — no media queries, no conditional classes in components.

**Focus ring is the hero state.** `box-shadow: var(--shadow-xbox-focus)` is the signature affordance. Never suppress it. Never replace it with a plain outline.

**Surfaces first, shadows with intent.** Content elevation should primarily come from surface steps (`--ui-surface-panel` → `--ui-surface-panel-strong`). For card-like content that already follows existing patterns, use approved tokens (e.g. `--ui-shadow-floating`) instead of ad-hoc shadow values. Overlays and focus states keep using their dedicated shadow tokens.

---

## 2. FOCUS STATE — THE ONE RULE YOU CANNOT SKIP

Every interactive element must implement this exact pattern when `data-focused="true"` (set automatically by `Focusable`):

```css
.my-component[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  /* optional: */ transform: scale(1.02);
  z-index: 10;
}
```

For child text elements, cascade the color explicitly:

```css
.my-component[data-focused='true'] .my-component__label {
  color: var(--ui-focus-text);
}
```

For border-based components (e.g. `SettingToggleRow`), use `border: 2px solid transparent` at rest and switch to `border-color: var(--color-focus-ring)` on focus — not box-shadow alone.

---

## 3. VISUAL HIERARCHY

Three layers per screen. Not two, not four.

| Layer | Role | Tokens |
|-------|------|--------|
| Primary | The one thing the user acts on | Largest size, `--color-text-primary`, full weight |
| Secondary | Supporting context, labels | `--ui-text-body-md` / `--ui-text-body-sm`, `--color-text-secondary` |
| Tertiary | Metadata, hints, timestamps | `--ui-text-body-sm` or smaller, `--color-text-disabled`, pushed to edges |

**Color is hierarchy.** `--color-text-primary` → `--color-text-secondary` → `--color-text-tertiary` → `--color-text-disabled`. Use the right level — don't make everything primary.

**Brand green is action, not decoration.** `--brand-primary` (#107C10) = buttons, active states, selected indicators, progress. Never use it as a background tint or decorative accent.

**Status colors encode data values.** Apply to the value or icon only — never to labels or row backgrounds:
- `--color-success` (#2BC24A) — connected, healthy
- `--color-warning` (#FDB900) — degraded, caution
- `--color-danger` (#FF5252) — error, destructive
- `--ui-status-positive` — online indicator dot

---

## 4. TYPOGRAPHY

Font stack always via `--ui-font-family` (`Segoe UI`, `WestEuropean`, `Microsoft YaHei`, sans-serif).

| Token | Size | Typical use |
|-------|------|-------------|
| `--ui-text-title-lg` | 31px | Page hero (e.g. Setting page group title uses `clamp(32px, 4vw, 44px)` with `font-weight: 900`) |
| `--ui-text-title-md` | 20px | Section headings, modal titles |
| `--ui-text-body-lg` | 19px | Console card description |
| `--ui-text-body-md` | 17px | Standard body, logout label |
| `--ui-text-body-xl` | 16px | Supporting body, setting row label (legacy name: this token is smaller than `--ui-text-body-md`) |
| `--ui-text-body-sm` | 13px | Captions, eyebrow labels, tabs |

Line heights: `--ui-line-height-tight` (1.1) for headings, `--ui-line-height-default` (1.2) for UI, `--ui-line-height-relaxed` (1.3) for body copy.

Eyebrow labels (section headers, modal eyebrows): `font-size: 12–13px`, `font-weight: 700`, `letter-spacing: 0.05–0.12em`, `text-transform: uppercase`. Color: `--brand-primary` for modal eyebrows, `--ui-page-text-soft` for action sheet eyebrows.

---

## 5. SPACING

Page inset: `--ui-page-inset` (density-responsive, never hardcode). Content padding: `padding: var(--ui-page-inset)` on `.app-shell__content`.

Component internal spacing uses `--ui-space-*` tokens. Common real values from components:

```
4px gap between inline items (score badge + text)
6px gap between shelf title and hint
8px gap between list items (action sheet)
12px gap between avatar and identity block
14px gap between icon and label (logout row)
16px padding on action sheet header, setting rows
24px padding on overlay panels, user menu
32px padding on modal panels
```

Use `gap` over `margin` for flex/grid children. Use `padding` for internal component breathing room.

---

## 6. MOTION

All component state transitions use `transition: all var(--ui-motion-fast)` or the explicit property list:

```css
transition:
  border-color var(--ui-motion-fast),
  background-color var(--ui-motion-fast),
  box-shadow var(--ui-motion-fast),
  transform var(--ui-motion-fast);
```

Overlay enter/leave: opacity `250ms ease` on the layer, transform `350ms cubic-bezier(0.2, 0, 0, 1)` on the panel. Direction depends on origin — action sheet slides from left, user menu slides from right.

Page/tab content transitions: `opacity + transform (translateY + scale(0.99))` at `250ms var(--ease-standard)`.

Scale feedback: hover `filter: brightness(1.03)` (GameCard) or `transform: scale(1.02)` (ConsoleCard, action sheet items). Never both at once.

---

## 7. DENSITY ADAPTATION

Design to `standard` first. Verify at `compact`. Density overrides live in `_theme-semantic.scss` — never in component files.

For layout that must change at `narrow`, use `:global(html[data-ui-density='narrow'])` inside the component's `<style scoped>`:

```css
:global(html[data-ui-density='narrow']) .my-panel {
  width: 100%;
  right: 0;
}
```

---

## 8. ANTI-PATTERNS

- Prefer tokens; avoid ad-hoc hex/px values in `<style scoped>`
- No `outline` suppression without replacing with `box-shadow: var(--shadow-xbox-focus)`
- No skeleton screens — use `BrandedLoading` component
- No toast popups — use inline status text
- No ad-hoc drop shadows on content cards — use surface hierarchy first, and only approved shadow tokens (e.g. `--ui-shadow-floating`) where existing patterns require it
- No `border-radius > 16px` on cards (16px is the max in use)
- No gradients in UI chrome (loading ring and brand aura are the only exceptions)
- No new interactive element without `Focusable` + `scope-id`
- No `position: fixed` overlay without `z-index: var(--z-overlay)` or higher

---

## 9. WORKFLOW

1. Read the closest existing component first — don't invent patterns
2. Design to `standard` density, verify at `compact`
3. Identify the 3 hierarchy layers before writing markup
4. Wire every interactive element as `Focusable` with `scope-id`
5. Implement `[data-focused='true']` state — this is non-negotiable
6. Use `--ui-motion-fast` for all transitions
7. Test `data-theme="light"` toggle

---

## 10. REFERENCE FILES

- **`references/tokens.md`** — Token values extracted from real source files, with actual usage context
- **`references/components.md`** — Patterns extracted from real component implementations
- **`references/platform-mapping.md`** — Vue 3 conventions, spatial nav wiring, overlay patterns, density overrides
