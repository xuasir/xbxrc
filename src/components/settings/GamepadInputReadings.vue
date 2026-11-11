<script setup lang="ts">
import type { RawPadState } from '@shared/gamepad/backend'
import type { LogicalPadStateDto } from '@shared/gamepad/contract'
import type { GameControls } from '@shared/gamepad/preferences'
import { applyGameControls } from '@shared/gamepad/preferences'
import { STANDARD_BUTTON_COUNT } from '@shared/gamepad/raw-input'
import { BUTTON_LABELS } from '@shared/gamepad/settings-model'
import { BUTTON_KEYS } from '@shared/gamepad/state'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

const props = withDefaults(defineProps<{ state: LogicalPadStateDto, raw?: RawPadState, controls: GameControls, layoutKey?: string, standard?: boolean, showStandard?: boolean, visualOnly?: boolean }>(), { showStandard: true })
const { t } = useI18n()
const observed = ref<Record<string, number[]>>({})
watch(() => props.raw, (raw) => {
  const layout = props.layoutKey ?? 'unknown'
  const indices = new Set(observed.value[layout] ?? [])
  raw?.buttons.forEach((value, index) => {
    if (value > 0.03 && (!props.standard || index >= STANDARD_BUTTON_COUNT))
      indices.add(index)
  })
  observed.value[layout] = [...indices].sort((a, b) => a - b)
}, { immediate: true })
const extraIndices = computed(() => observed.value[props.layoutKey ?? 'unknown'] ?? [])
const views = computed(() => [
  { label: t('gamepadSettings.standardInput'), state: props.state },
  { label: t('gamepadSettings.gameOutput'), state: applyGameControls(props.state, props.controls, props.raw, props.layoutKey, props.standard) },
].filter((_, index) => props.visualOnly ? index === 0 : props.showStandard || index > 0))
</script>

<template>
  <div class="gamepad-readings">
    <section v-for="view in views" :key="view.label" class="gamepad-readings__section">
      <h3 v-if="!visualOnly">
        {{ view.label }}
      </h3>
      <div v-if="!visualOnly" class="gamepad-readings__buttons">
        <span v-for="key in BUTTON_KEYS" :key="key" :data-pressed="view.state.buttons[key] > .5">{{ BUTTON_LABELS[key] }} {{ view.state.buttons[key].toFixed(2) }}</span>
      </div>
      <div class="gamepad-readings__sticks">
        <div v-for="side in ['leftStick', 'rightStick'] as const" :key="side">
          <strong>{{ t(`gamepadSettings.${side}`) }}</strong>
          <div class="gamepad-readings__plane" :aria-label="t(`gamepadSettings.${side}`)">
            <i :style="{ transform: `translate(${view.state[side].x * 34}px, ${view.state[side].y * 34}px)` }" />
          </div>
          <small>X {{ view.state[side].x.toFixed(2) }} · Y {{ view.state[side].y.toFixed(2) }}</small>
        </div>
      </div>
    </section>
    <section v-if="raw && !visualOnly" class="gamepad-readings__section">
      <h3>{{ t('gamepadSettings.rawInput') }}</h3>
      <p>{{ t('gamepadSettings.rawCounts', { buttons: raw.buttons.length, axes: raw.axes.length }) }}</p>
      <div class="gamepad-readings__buttons">
        <span v-for="(value, index) in raw.buttons" :key="index" :data-raw-button="index" :data-pressed="value > .5">
          {{ standard && index >= STANDARD_BUTTON_COUNT ? t('gamepadSettings.extraButton', { number: index - STANDARD_BUTTON_COUNT + 1 }) : t('gamepadSettings.rawButton', { index }) }} · {{ value.toFixed(2) }}
        </span>
      </div>
      <div class="gamepad-readings__buttons">
        <span v-for="(value, index) in raw.axes" :key="index">{{ t('gamepadSettings.rawAxis', { index }) }} · {{ value.toFixed(2) }}</span>
      </div>
      <p>{{ t('gamepadSettings.extraButtonHint') }}</p>
    </section>
    <div v-if="visualOnly && raw && extraIndices.length" class="gamepad-readings__buttons">
      <span v-for="index in extraIndices" :key="index" :data-raw-button="index" :data-pressed="(raw.buttons[index] ?? 0) > .5">{{ standard ? t('gamepadSettings.extraButton', { number: index - STANDARD_BUTTON_COUNT + 1 }) : t('gamepadSettings.rawButton', { index }) }} · {{ (raw.buttons[index] ?? 0).toFixed(2) }}</span>
    </div>
  </div>
</template>

<style scoped>
.gamepad-readings { display: grid; gap: 20px; min-width: 0; }
.gamepad-readings__section { display: grid; gap: 12px; }
.gamepad-readings h3 { margin: 0; color: var(--brand-primary); font-size: var(--ui-text-body-sm); }
.gamepad-readings p { margin: 0; color: var(--color-text-secondary); font-size: 12px; line-height: 1.5; }
.gamepad-readings__buttons { display: flex; flex-wrap: wrap; gap: 6px; }
.gamepad-readings__buttons span { padding: 6px 8px; border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-sm); background: var(--ui-settings-choice-bg); color: var(--color-text-secondary); font-size: 12px; font-variant-numeric: tabular-nums; }
.gamepad-readings__buttons [data-pressed='true'] { background: var(--brand-primary); color: var(--brand-on-primary); }
.gamepad-readings__sticks { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
.gamepad-readings__sticks > div { display: grid; justify-items: center; gap: 8px; font-size: 12px; }
.gamepad-readings__sticks strong { font-weight: 500; }
.gamepad-readings__sticks small { color: var(--color-text-secondary); font-variant-numeric: tabular-nums; }
.gamepad-readings__plane { display: grid; place-items: center; width: 80px; height: 80px; border: 1px solid var(--ui-border-subtle); border-radius: 50%; background: var(--ui-settings-choice-bg); }
.gamepad-readings__plane i { width: 12px; height: 12px; border-radius: 50%; background: var(--color-text-primary); }
</style>
