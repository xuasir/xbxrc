<script setup lang="ts">
import type { GamepadKeyboardControlDto, LogicalPadStateDto } from '@shared/gamepad/contract'
import type { GameControls } from '@shared/gamepad/preferences'
import type { ButtonKey } from '@shared/gamepad/state'
import { BUTTON_LABELS } from '@shared/gamepad/settings-model'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import controllerImage from '../../assets/ctrl/series-ctrl-transparent.png'

const props = withDefaults(defineProps<{ controls: GameControls, state: LogicalPadStateDto, scopeId?: string, disabled?: boolean, readonly?: boolean, editing?: boolean, mode?: 'mapping' | 'keyboard' | 'input' | 'sticks' | 'vibration', labels?: Partial<Record<ButtonKey, string>>, keyboardDirections?: Partial<Record<GamepadKeyboardControlDto, string>>, recording?: GamepadKeyboardControlDto | null, rumbling?: boolean, selected?: ButtonKey | null }>(), { mode: 'mapping' })
const emit = defineEmits<{ select: [source: ButtonKey], selectDirection: [control: GamepadKeyboardControlDto], closeEditor: [] }>()
const { t } = useI18n()
const highlighted = ref<ButtonKey | null>(null)
const canvas = ref<HTMLElement>()
const dropdown = ref<HTMLElement>()
const anchor = shallowRef<HTMLElement>()
const dropdownStyle = ref<Record<string, string>>({})
let resizeObserver: ResizeObserver | undefined
function placeDropdown(): void {
  if (!canvas.value || !anchor.value)
    return
  const parent = canvas.value.getBoundingClientRect()
  const button = anchor.value.getBoundingClientRect()
  const width = Math.min(264, parent.width - 8)
  const left = Math.max(4, Math.min(parent.width - width - 4, button.left - parent.left))
  dropdownStyle.value = { 'left': `${left}px`, 'top': `${button.bottom - parent.top + 8}px`, 'width': `${width}px`, '--anchor-x': `${Math.max(14, Math.min(width - 14, button.left + button.width / 2 - parent.left - left))}px` }
}
watch(() => props.editing, async (open) => {
  if (!open)
    return
  await nextTick()
  placeDropdown()
  await nextTick()
  dropdown.value?.scrollIntoView({ block: 'nearest' })
})
function outside(event: PointerEvent): void {
  if (props.editing && event.target instanceof Node && !dropdown.value?.contains(event.target) && !anchor.value?.contains(event.target))
    emit('closeEditor')
}
onMounted(() => {
  resizeObserver = new ResizeObserver(placeDropdown)
  if (canvas.value)
    resizeObserver.observe(canvas.value)
  document.addEventListener('pointerdown', outside)
})
onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  document.removeEventListener('pointerdown', outside)
})
const compact = computed(() => props.mode === 'sticks' || props.mode === 'vibration')
const canvasHeight = computed(() => compact.value ? 400 : 560)
// 热点使用图片像素，图片与连接线随同一画布缩放。
const imageBox = computed(() => ({ x: 267.5, y: compact.value ? 38.75 : 118.75, scale: 1.25 }))
const buttonCallouts: { key: ButtonKey, side: 'left' | 'right' | 'center', row: number, x: number, y: number }[] = [
  { key: 'home', side: 'center', row: 0, x: 185, y: 30 },
  { key: 'l2', side: 'left', row: 0, x: 83, y: 3 },
  { key: 'l1', side: 'left', row: 1, x: 83, y: 16 },
  { key: 'l3', side: 'left', row: 2, x: 90, y: 69 },
  { key: 'view', side: 'left', row: 3, x: 158, y: 70 },
  { key: 'dpadUp', side: 'left', row: 4, x: 137, y: 111 },
  { key: 'dpadLeft', side: 'left', row: 5, x: 117, y: 130 },
  { key: 'dpadDown', side: 'left', row: 6, x: 137, y: 149 },
  { key: 'dpadRight', side: 'left', row: 7, x: 157, y: 130 },
  { key: 'r2', side: 'right', row: 0, x: 289, y: 3 },
  { key: 'r1', side: 'right', row: 1, x: 289, y: 16 },
  { key: 'north', side: 'right', row: 2, x: 280, y: 46 },
  { key: 'west', side: 'right', row: 3, x: 255, y: 70 },
  { key: 'east', side: 'right', row: 4, x: 306, y: 70 },
  { key: 'south', side: 'right', row: 5, x: 281, y: 96 },
  { key: 'menu', side: 'right', row: 6, x: 212, y: 70 },
  { key: 'r3', side: 'right', row: 7, x: 232, y: 127 },
]
const callouts = computed(() => compact.value ? buttonCallouts.filter(item => ['l3', 'r3', ...(props.mode === 'vibration' ? ['l2', 'r2'] : [])].includes(item.key)).map(item => ({ ...item, row: ['l2', 'r2'].includes(item.key) ? 0 : 1, ...(props.mode === 'vibration' && ['l3', 'r3'].includes(item.key) ? { x: item.side === 'left' ? 76 : 298, y: 180 } : {}) })) : buttonCallouts)
const artStyle = computed(() => ({ left: '26.75%', top: `${imageBox.value.y / canvasHeight.value * 100}%`, width: '46.5%', height: `${322.5 / canvasHeight.value * 100}%` }))
function narrowRow(item: typeof buttonCallouts[number]): number {
  return compact.value ? props.mode === 'vibration' ? item.row + 2 : 2 : item.side === 'center' ? 2 : item.row + 3
}
function hitStyle(item: typeof buttonCallouts[number]) {
  const radius = props.mode === 'vibration' ? 32 : ['l3', 'r3'].includes(item.key) ? 24 : 11
  return { left: `${item.x / 372 * 100}%`, top: `${item.y / 258 * 100}%`, width: `${radius * 2 / 372 * 100}%`, height: `${(['l2', 'r2'].includes(item.key) ? 8 : radius * 2) / 258 * 100}%` }
}
function mapped(source: ButtonKey): ButtonKey {
  return props.controls.buttons[source]
}
const stickDirections = computed(() => ['leftStick', 'rightStick'].flatMap((side, index) => [{ direction: 'Up', arrow: '↑', dx: 0, dy: -42 }, { direction: 'Down', arrow: '↓', dx: 0, dy: 42 }, { direction: 'Left', arrow: '←', dx: -42, dy: 0 }, { direction: 'Right', arrow: '→', dx: 42, dy: 0 }].map(item => ({ ...item, control: `${side}${item.direction}` as GamepadKeyboardControlDto, x: (index ? 232 : 90) + item.dx, y: (index ? 127 : 69) + item.dy }))))
function target(source: ButtonKey): string {
  return props.labels?.[source] ?? (props.mode === 'input' ? props.state.buttons[source].toFixed(2) : BUTTON_LABELS[mapped(source)])
}
function sourceLabel(item: typeof buttonCallouts[number]): string {
  return props.mode === 'vibration' && ['l3', 'r3'].includes(item.key) ? item.side === 'left' ? 'L' : 'R' : BUTTON_LABELS[item.key]
}
function active(key: ButtonKey): boolean {
  return highlighted.value === key || props.selected === key || (props.mode === 'vibration' ? props.rumbling === true : props.state.buttons[key] > 0.5)
}
function line(item: typeof buttonCallouts[number]): string {
  if (item.side === 'center')
    return `500,105 500,125 ${imageBox.value.x + item.x * imageBox.value.scale},${imageBox.value.y + item.y * imageBox.value.scale}`
  const y = compact.value ? props.mode === 'vibration' ? 80 + item.row * 190 : 200 : 37 + item.row * 65
  const start = compact.value ? item.side === 'left' ? 285 : 715 : item.side === 'left' ? 255 : 745
  const bend = compact.value ? item.side === 'left' ? 295 : 705 : item.side === 'left' ? 265 : 735
  return `${start},${y} ${bend},${y} ${imageBox.value.x + item.x * imageBox.value.scale},${imageBox.value.y + item.y * imageBox.value.scale}`
}
function select(key: ButtonKey, event: MouseEvent): void {
  if (!props.readonly && !props.disabled && !props.recording) {
    anchor.value = event.currentTarget as HTMLElement
    emit('select', key)
    void nextTick(placeDropdown)
  }
}
</script>

<template>
  <div class="gamepad-mapping" :data-mode="mode" :aria-label="t('gamepadSettings.mappingDiagram')">
    <div v-if="!compact" class="gamepad-mapping__legend">
      <span>{{ mode === 'keyboard' ? t('gamepadSettings.keyboardBindingLegend') : mode === 'input' ? t('gamepadSettings.standardInput') : mode === 'sticks' ? t('gamepadSettings.stickPreviewLegend') : mode === 'vibration' ? t('gamepadSettings.vibrationPreviewLegend') : t('gamepadSettings.mappingLegend') }}</span><span v-if="mode === 'mapping'">{{ t('gamepadSettings.selectButtonHint') }}</span>
    </div>
    <div ref="canvas" class="gamepad-mapping__canvas" :data-compact="compact" :data-rumbling="rumbling" :data-editing="editing">
      <div class="gamepad-mapping__visual">
        <svg class="gamepad-mapping__art" viewBox="0 0 372 258" :style="artStyle" aria-hidden="true">
          <image :href="controllerImage" width="372" height="258" />
          <circle v-for="item in callouts" :key="item.key" class="gamepad-mapping__hotspot" :data-active="active(item.key)" :cx="item.x" :cy="item.y" :r="mode === 'vibration' ? 32 : ['l3', 'r3'].includes(item.key) ? 24 : 11" />
          <g v-if="mode === 'sticks'" class="gamepad-mapping__stick-state">
            <g v-for="item in callouts" :key="item.key">
              <circle :cx="item.x" :cy="item.y" :r="24 * (item.key === 'l3' ? controls.leftDeadzone : controls.rightDeadzone)" />
              <circle :cx="item.x + (item.key === 'l3' ? state.leftStick.x : state.rightStick.x) * 20" :cy="item.y + (item.key === 'l3' ? state.leftStick.y : state.rightStick.y) * 20" r="3" />
            </g>
          </g>
        </svg>
        <div v-if="!readonly && scopeId" class="gamepad-mapping__hit-area" :style="artStyle">
          <Focusable v-for="item in callouts" :id="`${scopeId}.hotspot.${item.key}`" :key="item.key" as="button" :disabled="disabled || editing" class="gamepad-mapping__hit" :style="hitStyle(item)" :aria-label="t('gamepadSettings.editOnController', { button: sourceLabel(item) })" @mouseenter="highlighted = item.key" @mouseleave="highlighted = null" @focus="highlighted = item.key" @blur="highlighted = null" @click="select(item.key, $event)" />
          <template v-if="mode === 'keyboard'">
            <Focusable v-for="item in stickDirections" :id="`${scopeId}.direction.${item.control}`" :key="item.control" as="button" class="gamepad-mapping__direction" :disabled="disabled" :data-recording="recording === item.control" :style="{ left: `${item.x / 372 * 100}%`, top: `${item.y / 258 * 100}%` }" :aria-label="`${t(`gamepadSettings.${item.control}`)} · ${keyboardDirections?.[item.control] ?? ''}`" @click="!recording && emit('selectDirection', item.control)">
              <span>{{ item.arrow }}</span><small v-if="mode === 'keyboard'">{{ keyboardDirections?.[item.control] }}</small>
            </Focusable>
          </template>
        </div>
        <svg class="gamepad-mapping__lines" :viewBox="`0 0 1000 ${canvasHeight}`" aria-hidden="true"><g v-for="item in callouts" :key="item.key" :data-active="active(item.key)"><polyline :points="line(item)" /><circle :cx="imageBox.x + item.x * imageBox.scale" :cy="imageBox.y + item.y * imageBox.scale" r="3" /></g></svg>
        <Focusable v-for="item in callouts" :id="readonly ? undefined : `${scopeId}.physical.${item.key}`" :key="item.key" :as="readonly ? 'div' : 'button'" :scope-id="readonly ? undefined : scopeId" :disabled="disabled || readonly" class="gamepad-mapping__callout" :class="`gamepad-mapping__callout--${item.side}`" :style="{ '--row': item.row, '--narrow-row': narrowRow(item) }" :data-changed="mode === 'mapping' && target(item.key) !== BUTTON_LABELS[item.key]" :data-pressed="active(item.key)" :aria-label="t('gamepadSettings.mappingPair', { source: sourceLabel(item), target: target(item.key) })" @mouseenter="highlighted = item.key" @mouseleave="highlighted = null" @focus="highlighted = item.key" @blur="highlighted = null" @click="select(item.key, $event)">
          <span class="gamepad-mapping__source" :data-button="item.key">{{ sourceLabel(item) }}</span>
          <svg v-if="!compact" class="gamepad-mapping__arrow" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M13 8H3m4-4L3 8l4 4" /></svg>
          <span class="gamepad-mapping__target" :title="target(item.key)"><strong>{{ target(item.key) }}</strong></span>
        </Focusable>
      </div>
      <div v-if="editing" ref="dropdown" class="gamepad-mapping__editor" :style="dropdownStyle">
        <slot name="editor" />
      </div>
    </div>
    <div v-if="$slots.extras" class="gamepad-mapping__extras">
      <slot name="extras" />
    </div>
    <p v-if="mode === 'mapping'" class="gamepad-mapping__system">
      {{ t('gamepadSettings.xboxCaptureSupport') }}
    </p>
  </div>
</template>

<style scoped>
.gamepad-mapping { container-type: inline-size; min-width: 0; width: 100%; max-width: 840px; margin-inline: auto; }
.gamepad-mapping__legend { display: flex; flex-wrap: wrap; justify-content: space-between; gap: var(--space-8); color: var(--color-text-secondary); font-size: 12px; padding-bottom: var(--space-12); }
.gamepad-mapping__legend span + span { color: var(--color-text-tertiary); }
.gamepad-mapping__canvas { position: relative; }
.gamepad-mapping__visual { position: relative; aspect-ratio: 1000 / 560; }
.gamepad-mapping__canvas[data-compact='true'] .gamepad-mapping__visual { aspect-ratio: 1000 / 400; }
.gamepad-mapping__art { position: absolute; overflow: visible; filter: drop-shadow(0 18px 16px rgb(0 0 0 / 18%)); }
.gamepad-mapping__hit-area { position: absolute; pointer-events: none; }
.gamepad-mapping__hit { position: absolute; transform: translate(-50%, -50%); padding: 0; border: 0; border-radius: 50%; background: transparent; pointer-events: auto; cursor: pointer; }
.gamepad-mapping__hit[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.gamepad-mapping__hit:disabled { pointer-events: none; }
.gamepad-mapping__direction { position: absolute; transform: translate(-50%, -50%); display: grid; place-items: center; min-width: 28px; min-height: 28px; padding: 2px 4px; border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-sm); background: var(--ui-surface-overlay); color: var(--color-text-primary); font-size: 11px; pointer-events: auto; }
.gamepad-mapping__direction small { font-size: 9px; color: var(--brand-primary); }
.gamepad-mapping__direction[data-focused='true'], .gamepad-mapping__direction[data-recording='true'] { box-shadow: var(--shadow-xbox-focus); }
.gamepad-mapping__stick-state { fill: color-mix(in srgb, var(--brand-primary) 30%, transparent); stroke: var(--brand-primary); stroke-width: 1; }
.gamepad-mapping__editor { position: absolute; z-index: 10; filter: drop-shadow(0 10px 20px rgb(0 0 0 / 20%)); }
.gamepad-mapping__editor::before { content: ''; position: absolute; left: var(--anchor-x); top: -5px; width: 10px; height: 10px; background: var(--ui-surface-overlay); border-top: 1px solid var(--ui-border-subtle); border-left: 1px solid var(--ui-border-subtle); transform: translateX(-50%) rotate(45deg); }
.gamepad-mapping__extras { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-8); margin-top: var(--space-12); }
.gamepad-mapping__hotspot { fill: none; stroke: transparent; stroke-width: 2; }
.gamepad-mapping__hotspot[data-active='true'] { fill: color-mix(in srgb, var(--brand-primary) 20%, transparent); stroke: var(--brand-primary); }
.gamepad-mapping__lines { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
.gamepad-mapping__lines polyline { fill: none; stroke: var(--color-text-tertiary); stroke-width: 1; opacity: .35; }
.gamepad-mapping__lines circle { fill: var(--color-text-tertiary); opacity: .6; }
.gamepad-mapping__lines [data-active='true'] polyline { stroke: var(--brand-primary); stroke-width: 1.5; opacity: .8; }
.gamepad-mapping__lines [data-active='true'] circle { fill: var(--brand-primary); opacity: 1; }
.gamepad-mapping__callout { position: absolute; top: calc((37 + var(--row) * 65) / 560 * 100%); transform: translateY(-50%); width: 24%; display: flex; align-items: center; gap: var(--space-8); min-height: 44px; padding: var(--space-4) var(--space-8); border: 1px solid transparent; border-radius: var(--ui-radius-md); background: transparent; color: var(--color-text-primary); font-size: var(--ui-text-body-sm); text-align: left; }
.gamepad-mapping__callout--left { left: 0; }
.gamepad-mapping__callout--left { justify-content: flex-end; text-align: right; }
.gamepad-mapping__callout--right { right: 0; }
.gamepad-mapping__callout--center { left: 50%; top: 12.5%; width: 27%; transform: translate(-50%, -50%); }
.gamepad-mapping__source { display: grid; place-items: center; flex: 0 0 30px; height: 30px; border-radius: var(--ui-radius-sm); background: var(--ui-settings-choice-bg); color: var(--color-text-secondary); font-size: 12px; font-weight: var(--ui-font-weight-semibold); }
.gamepad-mapping__source[data-button='south'] { color: #63bc52; }
.gamepad-mapping__source[data-button='east'] { color: #dc646a; }
.gamepad-mapping__source[data-button='west'] { color: #559de0; }
.gamepad-mapping__source[data-button='north'] { color: #b49b3e; }
.gamepad-mapping__source[data-button='home'] { flex-basis: 40px; }
.gamepad-mapping__arrow { flex: 0 0 12px; height: 12px; color: var(--color-text-tertiary); }
.gamepad-mapping__target { min-width: 0; display: grid; gap: 4px; }
.gamepad-mapping__target strong { font-weight: var(--ui-font-weight-medium); white-space: normal; line-height: 1.15; overflow-wrap: anywhere; }
.gamepad-mapping__callout[data-changed='true'] .gamepad-mapping__target strong { color: var(--brand-primary); }
.gamepad-mapping__callout[data-focused='true'] { background: var(--ui-material-selected); box-shadow: var(--shadow-xbox-focus); }
.gamepad-mapping__callout[data-pressed='true'] .gamepad-mapping__source { color: var(--brand-primary); background: var(--ui-material-selected); }
.gamepad-mapping__callout:hover { background: var(--ui-material-hover); }
.gamepad-mapping__callout:disabled { opacity: 1; }
.gamepad-mapping__canvas[data-compact='true'] .gamepad-mapping__callout { top: 50%; width: 27%; }
.gamepad-mapping[data-mode='vibration'] .gamepad-mapping__callout { top: calc((80 + var(--row) * 190) / 400 * 100%); }
.gamepad-mapping__system { margin: var(--space-8) 0 0; color: var(--color-text-tertiary); font-size: 12px; text-align: center; }
@media(min-width:701px) and (max-height:750px) { .gamepad-mapping { max-width: 640px; } }
@media(min-width:701px) and (max-height:650px) { .gamepad-mapping { max-width: 520px; } }
@container (max-width:750px) { .gamepad-mapping__callout { gap: var(--space-4); min-height: 32px; padding: 2px var(--space-4); font-size: 13px; } .gamepad-mapping__source { flex-basis: 26px; height: 26px; font-size: 11px; } }
@container (min-width:481px) and (max-width:750px) { .gamepad-mapping__target strong { font-size: 12px; } }
@container (max-width:480px) {
  .gamepad-mapping__canvas, .gamepad-mapping__canvas[data-compact='true'] { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--space-8); aspect-ratio: auto; }
  .gamepad-mapping__visual { display: contents; }
  .gamepad-mapping__art { position: static; grid-column: 1 / -1; grid-row: 1; width: 100% !important; height: 210px !important; }
  .gamepad-mapping__hit-area { left: 50% !important; top: 105px !important; width: min(100%, 302.8px) !important; height: auto !important; aspect-ratio: 372 / 258; transform: translate(-50%, -50%); }
  .gamepad-mapping__lines { display: none; }
  .gamepad-mapping__callout { position: static; grid-row: var(--narrow-row); transform: none; width: 100%; min-height: 42px; background: var(--ui-settings-item-bg); }
  .gamepad-mapping__callout--left { grid-column: 1; }
  .gamepad-mapping__callout--left { justify-content: flex-start; text-align: left; }
  .gamepad-mapping__callout--right { grid-column: 2; }
  .gamepad-mapping__callout--center { grid-column: 1 / -1; }
  .gamepad-mapping__canvas[data-compact='true'] .gamepad-mapping__callout { width: 100%; }
}
</style>
