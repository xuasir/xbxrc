<script setup lang="ts">
import type { StreamShortcut } from '@shared/gamepad/preferences'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import xboxLogoIcon from '../../assets/nav/xbox-logo.svg'
import diagnosticsIcon from '../../assets/stream/stream-diagnostics.svg'
import SpatialNavIconButton from '../navigation/SpatialNavIconButton.vue'

const props = defineProps<{ scopeId: string, bindings: Record<StreamShortcut, string>, recording: StreamShortcut | null, busy: boolean }>()
const emit = defineEmits<{ select: [target: StreamShortcut] }>()
const { t } = useI18n()
function label(target: StreamShortcut): string {
  return t(`gamepadSettings.${target === 'menu' ? 'streamMenuShortcut' : 'diagnosticsMenuShortcut'}`)
}
function select(target: StreamShortcut): void {
  if (!props.recording && !props.busy)
    emit('select', target)
}
</script>

<template>
  <div class="stream-shortcut-diagram" :aria-label="t('gamepadSettings.shortcutDiagram')">
    <div class="stream-page__chrome-container">
      <SpatialNavIconButton :id="`${scopeId}.hotspot.menu`" class="stream-page__chrome" :scope-id="scopeId" :label="label('menu')" :icon-src="xboxLogoIcon" round :active="recording === 'menu'" :busy="busy" @click="select('menu')" />
    </div>
    <div class="stream-page__diagnostics-container">
      <SpatialNavIconButton :id="`${scopeId}.hotspot.diagnostics`" class="stream-page__chrome" :scope-id="scopeId" :label="label('diagnostics')" :icon-src="diagnosticsIcon" round :active="recording === 'diagnostics'" :busy="busy" @click="select('diagnostics')" />
    </div>
    <div v-for="target in ['menu', 'diagnostics'] as const" :key="target" class="stream-shortcut-diagram__mapping" :class="`stream-shortcut-diagram__mapping--${target}`">
      <Focusable :id="`${scopeId}.target.${target}`" as="button" :scope-id="scopeId" :busy="busy" class="stream-shortcut-diagram__target" :data-recording="recording === target" :aria-label="t('gamepadSettings.shortcutMappingPair', { menu: label(target), binding: bindings[target] })" @click="select(target)">
        <span>{{ label(target) }}</span>
        <strong>{{ bindings[target] }}</strong>
      </Focusable>
      <slot name="target-action" :target="target" />
    </div>
  </div>
</template>

<style scoped src="../stream/stream-menu-chrome.css"></style>

<style scoped>
.stream-shortcut-diagram { position: relative; isolation: isolate; width: 100%; height: 100%; min-width: 0; min-height: 220px; }
.stream-shortcut-diagram__mapping { position: absolute; left: calc(24px + var(--ui-size-control-xl) + 24px); display: flex; align-items: center; gap: var(--space-8); width: min(280px, calc(100% - 24px - var(--ui-size-control-xl) - 48px)); }
.stream-shortcut-diagram__mapping::before { content: ''; position: absolute; top: 50%; left: -20px; width: 16px; border-top: 1px solid var(--color-text-tertiary); opacity: 0.45; pointer-events: none; }
.stream-shortcut-diagram__mapping--menu { top: calc(24px + var(--ui-size-control-xl) / 2); transform: translateY(-50%); }
.stream-shortcut-diagram__mapping--diagnostics { bottom: calc(24px + var(--ui-size-control-xl) / 2); transform: translateY(50%); }
.stream-shortcut-diagram__target { display: flex; flex: 1; flex-direction: column; gap: var(--space-4); min-width: 0; min-height: 44px; padding: var(--space-8); border: 1px solid transparent; border-radius: var(--ui-radius-md); background: var(--ui-settings-item-bg); color: var(--color-text-primary); text-align: left; overflow-wrap: anywhere; }
.stream-shortcut-diagram__target span { font-size: var(--ui-text-body-sm); font-weight: var(--ui-font-weight-medium); }
.stream-shortcut-diagram__target strong { font-size: 12px; font-weight: var(--ui-font-weight-medium); color: var(--color-text-secondary); }
.stream-shortcut-diagram__target:hover { background: var(--ui-material-hover); }
.stream-shortcut-diagram__target[data-focused='true'], .stream-shortcut-diagram__target[data-recording='true'] { background: var(--ui-material-selected); box-shadow: var(--shadow-xbox-focus); }
</style>
