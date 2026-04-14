# XBX Design System — Tokens

Token values extracted from `src/styles/tokens.css`, `src/styles/_theme-foundation.scss`, and `src/styles/_theme-semantic.scss`. These are the values actually in use — not aspirational.

---

## 1. BRAND

```css
--brand-primary:        #107c10   /* Xbox green — buttons, active states, selected */
--brand-primary-strong: #158f15   /* Hover/focus variant */
--brand-accent:         #2bc24a   /* Bright accent, success, online indicator */
--brand-on-primary:     #ffffff   /* Text on brand surfaces */
```

---

## 2. NEUTRAL SCALE

Full gray ramp from `tokens.css`. Used directly in `color-mix()` expressions.

```css
--neutral-0:    #000000
--neutral-50:   #050505
--neutral-100:  #0a0a0b
--neutral-150:  #0e0e10
--neutral-200:  #121215
--neutral-250:  #18181b
--neutral-300:  #1f1f23
--neutral-350:  #27272c
--neutral-400:  #2f2f35
--neutral-500:  #3c3c43
--neutral-600:  #55555e
--neutral-700:  #7c7c88
--neutral-800:  #a9a9b2
--neutral-900:  #d6d6db
--neutral-1000: #ffffff
```

---

## 3. SEMANTIC COLOR TOKENS

### Text
```css
--color-text-primary:   var(--neutral-1000)   /* #ffffff dark */
--color-text-secondary: #d1d1d6
--color-text-tertiary:  #a1a1aa
--color-text-disabled:  #71717a
--color-text-inverse:   #050505               /* text on light surfaces */
--color-text-on-media:  #ffffff               /* always white, over images */
```

In components, `--ui-page-text` and `--ui-page-text-soft` are also used (theme-semantic aliases). Prefer `--color-text-*` for new code.

### Surfaces
```css
--color-bg:           #0f0f10   /* page background */
--color-bg-elevated:  #1a1b1e
--color-surface-0:    #0f0f10
--color-surface-1:    #2b2b2b   /* panels, cards */
--color-surface-2:    #3a3a3a   /* raised surfaces */
--color-surface-3:    #4a4a4a   /* highest elevation */
```

Overlay surfaces use `--ui-surface-overlay` (= `#1a1b1e` dark) and `--ui-surface-info-panel` (stream window only).

### Borders
```css
--color-border-subtle: rgba(255,255,255,0.06)   /* decorative, GameCard */
--color-border:        rgba(255,255,255,0.12)   /* standard */
--color-border-strong: rgba(255,255,255,0.20)   /* intentional */
--color-divider:       rgba(255,255,255,0.08)   /* list dividers */
--ui-border-subtle:    (theme-semantic alias, used in most components)
```

### Interaction States
```css
--color-state-hover:    rgba(255,255,255,0.08)
--color-state-pressed:  rgba(255,255,255,0.12)
--color-state-selected: rgba(16,124,16,0.28)    /* brand green tint */
--color-state-disabled: rgba(255,255,255,0.04)
```

### Status
```css
--color-success: #2bc24a
--color-warning: #fdb900
--color-danger:  #ff5252
--color-info:    #52aaff
--ui-status-positive: (= --brand-accent, online dot)
--ui-status-danger:   (= --color-danger)
```

### Focus
```css
--color-focus-ring:       #ffffff              /* outer ring color */
--color-focus-ring-outer: transparent          /* gap ring */
--color-focus-bg:         rgba(255,255,255,0.12)
--color-focus-bg-strong:  rgba(255,255,255,0.20)  /* used in most [data-focused] */
--ui-focus-text:          #000000              /* text color when focused */

--shadow-xbox-focus: 0 0 0 2px var(--color-bg),
                     0 0 0 5px var(--color-focus-ring),
                     0 12px 24px rgba(0,0,0,0.5)
--shadow-xbox-panel: 0 24px 48px rgba(0,0,0,0.8),
                     inset 0 1px 1px rgba(255,255,255,0.1)
```

---

## 4. TYPOGRAPHY

```css
--ui-font-family: 'Segoe UI', 'WestEuropean', 'Microsoft YaHei', sans-serif

/* Size tokens */
--ui-text-title-lg:  31px    /* page hero headings */
--ui-text-title-md:  20px    /* section headings, modal titles */
--ui-text-body-lg:   19px    /* console card description */
--ui-text-body-md:   17px    /* standard body, logout label */
--ui-text-body-xl:   16px    /* setting row label, supporting body */
--ui-text-body-sm:   13px    /* captions, eyebrows, tabs */

/* Line height */
--ui-line-height-tight:   1.1   /* headings */
--ui-line-height-default: 1.2   /* UI labels */
--ui-line-height-relaxed: 1.3   /* body copy */

/* Weight */
--ui-font-weight-medium:   500
--ui-font-weight-semibold: 600
--ui-font-weight-bold:     700
--font-weight-black:       900   /* setting page hero title */
```

Real usage note: Setting page group title uses `clamp(32px, 4vw, 44px)` with `font-weight: 900` and `letter-spacing: -0.02em` — this is a deliberate exception for the hero moment.

---

## 5. SPACING

```css
/* Named scale (from _theme-foundation.scss) */
--ui-space-2xs: 2px
--ui-space-xs:  4px
--ui-space-sm:  7px
--ui-space-md:  9px
--ui-space-lg:  13px
--ui-space-xl:  16px
--ui-space-2xl: 19px
--ui-space-3xl: 20px
--ui-space-4xl: 24px
--ui-space-5xl: 27px

/* Layout */
--ui-page-inset:      16px (standard) → 24px (comfortable) → 12px (compact) → 10px (narrow)
--ui-page-stack-gap:  var(--ui-space-xl)
--ui-size-nav-height: density-responsive
--ui-size-control-sm: small control height
--ui-size-control-lg: standard button height
--ui-size-control-xl: large control height
```

---

## 6. RADIUS

```css
--ui-radius-sm:   4px    /* technical elements, inline select segments */
--ui-radius-md:   8px    /* buttons, inputs, small cards */
--ui-radius-lg:   12px   /* cards */
--ui-radius-pill: 999px  /* toggles, tags, avatar, close buttons */
```

Overlay panels (action sheet, user menu) use `border-radius: 16px` directly — this is the practical max.

---

## 7. MOTION

```css
--ui-motion-fast: (defined in _theme-semantic.scss, ~150-220ms ease)

/* From tokens.css */
--ease-standard:  cubic-bezier(0.2, 0, 0, 1)
--ease-emphasized: cubic-bezier(0.2, 0, 0, 1.2)
--ease-linear:    linear

--duration-80:  80ms
--duration-120: 120ms
--duration-160: 160ms
--duration-200: 200ms
--duration-240: 240ms
--duration-320: 320ms

--scale-hover:   1.03
--scale-focus:   1.04
--scale-pressed: 0.99
```

Overlay transitions: layer opacity `250ms ease`, panel transform `350ms var(--ease-standard)`.

---

## 8. SHADOWS

```css
--shadow-1: 0 4px 16px rgba(0,0,0,0.35)
--shadow-2: 0 10px 30px rgba(0,0,0,0.45)
--shadow-3: 0 18px 60px rgba(0,0,0,0.55)

--ui-shadow-floating: 0 12px 32px color-mix(in srgb, var(--neutral-0) 24%, transparent)
--ui-shadow-overlay:  (defined per density, e.g. 0 19px 44px rgba(0,0,0,0.42))
```

Content cards use `--ui-shadow-floating`. Overlays use `--ui-shadow-overlay`. Focus uses `--shadow-xbox-focus`.

---

## 9. Z-INDEX

```css
--z-base:    0
--z-nav:     10
--z-sticky:  20
--z-overlay: 100   /* action sheets, user menu, gamepad card */
--z-drawer:  120
--z-modal:   140   /* SettingModalShell */
--z-toast:   200   /* avoid — use inline status */
```

---

## 10. COMPONENT-LEVEL TOKENS (KEY SUBSET)

### Buttons / Actions
```css
--btn-radius:          var(--ui-radius-md)
--btn-height-md:       var(--ui-size-control-lg)
--btn-height-lg:       var(--ui-size-control-xl)
--btn-primary-bg:      var(--brand-primary)
--btn-primary-bg-hover: var(--brand-primary-strong)
--btn-primary-text:    var(--brand-on-primary)
--btn-border:          var(--ui-border-subtle)
```

### Settings
```css
--settings-item-height: var(--ui-settings-row-min-height)   /* 56px → 44px compact */
--settings-item-radius: var(--ui-radius-md)
--ui-settings-row-label-size:       16px → 12px compact
--ui-settings-row-description-size: 14px → 10px compact
--ui-settings-toggle-track-width:   52px → 42px compact
--ui-settings-toggle-track-height:  24px → 20px compact
--ui-settings-toggle-thumb-size:    16px → 12px compact
--ui-settings-toggle-thumb-offset:  26px → 20px compact
```

### Cards
```css
/* Game card */
--ui-game-card-size:           176px → 152px (compact) → 140px (narrow)
--ui-game-card-radius:         12px → 10px (compact)
--ui-game-card-title-font-size: 15px → 13px (compact)

/* Console card */
--ui-console-card-width:       304px → 252px (compact) → 228px (narrow)
--ui-console-card-radius:      16px → 14px (compact) → 12px (narrow)
--ui-console-card-title-size:  25px → 22px (compact)
```

### Shelf / Rail
```css
--shelf-row-gap:       var(--ui-rail-gap)          /* 14px → 12px compact */
--shelf-title-gap:     var(--ui-rail-copy-gap)      /* 6px → 4px narrow */
--shelf-scroll-padding: var(--ui-rail-padding-inline) /* 16px → 12px compact */
--ui-rail-title-size:  22px → 18px (compact) → 17px (narrow)
--ui-rail-hint-size:   13px → 12px (compact/narrow)
```
