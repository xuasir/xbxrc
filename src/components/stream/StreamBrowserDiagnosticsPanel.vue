<script setup lang="ts">
import type { StreamDiagnosticGroup } from '../../streaming/stream-panel-view-models'
import type { StreamBrowserDiagnosticsViewModel, StreamEnhancementMountState } from '../../streaming/types'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { buildDiagnosticClipboardSnapshot, DIAGNOSTIC_GROUPS, diagnosticGroupRows } from '../../streaming/stream-panel-view-models'

const props = defineProps<{
  visible: boolean
  mount: StreamEnhancementMountState
  model: StreamBrowserDiagnosticsViewModel
  group: StreamDiagnosticGroup
}>()
const emit = defineEmits<{
  close: []
  groupChange: [group: StreamDiagnosticGroup]
}>()
const { t } = useI18n()
const scopeId = 'stream.diagnostic-details'
const panelVisible = computed(() => props.visible && props.mount.phase === 'mounted')
const rows = computed(() => diagnosticGroupRows(props.model, props.group))
const copyState = ref<'idle' | 'copied' | 'failed'>('idle')

function handleEscape(event: KeyboardEvent): void {
  if (event.defaultPrevented || !navigationEngine.isTopmostScope(scopeId))
    return
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('close')
  }
}
watch(panelVisible, (visible) => {
  window.removeEventListener('keydown', handleEscape, true)
  copyState.value = 'idle'
  if (visible)
    window.addEventListener('keydown', handleEscape, true)
}, { immediate: true })
onBeforeUnmount(() => window.removeEventListener('keydown', handleEscape, true))

async function copySnapshot(): Promise<void> {
  try {
    await navigator.clipboard.writeText(buildDiagnosticClipboardSnapshot(props.model))
    copyState.value = 'copied'
  }
  catch {
    copyState.value = 'failed'
  }
}
</script>

<template>
  <Transition name="stream-diagnostic">
    <FocusScope
      v-if="panelVisible"
      :id="scopeId"
      as="aside"
      class="stream-diagnostic"
      :active="panelVisible"
      :default-focus-id="`${scopeId}.${group}`"
      :aria-label="t('streamPage.diagnosticsPanel.browser.title')"
    >
      <header class="stream-diagnostic__header">
        <h2>{{ t('streamPage.diagnosticsPanel.browser.title') }}</h2>
        <Focusable :id="`${scopeId}.close`" :scope-id="scopeId" as="button" type="button" class="stream-diagnostic__button stream-diagnostic__close" :aria-label="t('streamPage.diagnosticsPanel.close')" @click="emit('close')">
          ×
        </Focusable>
      </header>
      <div class="stream-diagnostic__tabs" role="tablist" :aria-label="t('streamPage.diagnosticsPanel.groupsLabel')">
        <Focusable
          v-for="tab in DIAGNOSTIC_GROUPS"
          :id="`${scopeId}.${tab}`"
          :key="tab"
          :scope-id="scopeId"
          as="button"
          type="button"
          role="tab"
          class="stream-diagnostic__button stream-diagnostic__tab"
          :aria-selected="tab === group"
          :aria-controls="`${scopeId}.content`"
          @click="emit('groupChange', tab)"
        >
          {{ t(`streamPage.diagnosticsPanel.groups.${tab}`) }}
        </Focusable>
      </div>
      <div :id="`${scopeId}.content`" :key="group" class="stream-diagnostic__rows" role="tabpanel" :aria-labelledby="`${scopeId}.${group}`" data-nav-scroll-container>
        <Focusable
          v-for="row in rows"
          :id="`${scopeId}.row.${row.key}`"
          :key="row.key"
          :scope-id="scopeId"
          as="div"
          class="stream-diagnostic__row ui-focus-outline"
          tabindex="0"
        >
          <span class="stream-diagnostic__label">{{ t(`streamPage.diagnosticsPanel.browser.fields.${row.key}`) }}</span>
          <span class="stream-diagnostic__value">{{ row.value }}</span>
        </Focusable>
      </div>
      <p class="stream-diagnostic__hint">
        {{ t(`streamPage.diagnosticsPanel.hints.${group}`) }}
      </p>
      <footer class="stream-diagnostic__footer">
        <Focusable :id="`${scopeId}.copy`" :scope-id="scopeId" as="button" type="button" class="stream-diagnostic__button stream-diagnostic__copy" @click="copySnapshot">
          {{ t('streamPage.diagnosticsPanel.copy') }}
        </Focusable>
        <span aria-live="polite" class="stream-diagnostic__copy-state">{{ copyState === 'idle' ? '' : t(`streamPage.diagnosticsPanel.${copyState}`) }}</span>
      </footer>
    </FocusScope>
  </Transition>
</template>

<style scoped>
.stream-diagnostic {
  position: absolute;
  top: var(--ui-stream-diagnostic-top);
  left: auto;
  right: var(--ui-stream-metrics-inset);
  bottom: var(--ui-stream-diagnostic-bottom);
  z-index: var(--z-stream-diagnostics);
  width: min(var(--ui-stream-diagnostic-width), calc(100% - var(--ui-stream-metrics-inset) * 2));
  padding: var(--ui-space-xl);
  border-radius: var(--ui-radius-lg);
  border: 1px solid var(--ui-border-subtle);
  background: var(--ui-stream-diagnostic-bg);
  color: var(--ui-page-text);
  font-family: var(--ui-font-family);
  font-size: var(--ui-text-body-sm);
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-sm);
  pointer-events: auto;
}
.stream-diagnostic__header { display: flex; align-items: center; justify-content: space-between; gap: var(--ui-space-md); }
.stream-diagnostic__header h2 { margin: 0; font-size: var(--ui-text-title-md); font-weight: var(--ui-font-weight-bold); }
.stream-diagnostic__tabs { display: flex; gap: 12px; padding: 8px; margin-inline: -8px; }
.stream-diagnostic__button {
  min-height: var(--ui-stream-diagnostic-control-height);
  padding: var(--ui-space-sm);
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: transparent;
  color: var(--ui-page-text);
  font: inherit;
  transition: background-color var(--ui-motion-fast), box-shadow var(--ui-motion-fast), border-color var(--ui-motion-fast);
}
.stream-diagnostic__close { min-width: var(--ui-stream-diagnostic-control-height); font-size: var(--ui-text-title-md); }
.stream-diagnostic__tab { flex: 1; min-width: 0; white-space: nowrap; }
.stream-diagnostic__button:hover { background: var(--ui-material-hover); }
.stream-diagnostic__tab[aria-selected='true'] { background: var(--ui-choice-selected-bg); }
.stream-diagnostic__rows { flex: 1; min-height: 0; overflow-y: auto; overscroll-behavior: contain; padding: 12px; margin-inline: -12px; scroll-padding: 12px; animation: ui-content-enter 160ms ease-out; }
.stream-diagnostic__row { --focus-ring-radius: var(--ui-radius-sm); display: flex; align-items: baseline; justify-content: space-between; gap: var(--ui-space-lg); padding: var(--ui-space-md) var(--ui-space-xs); border: 1px solid transparent; border-bottom-color: var(--ui-border-subtle); border-radius: var(--focus-ring-radius); }
.stream-diagnostic__label { flex: 1; color: var(--ui-page-text-soft); }
.stream-diagnostic__value { max-width: 60%; text-align: right; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
.stream-diagnostic__hint { margin: 0; color: var(--ui-page-text-soft); font-size: var(--ui-stream-metrics-font-size); line-height: var(--ui-line-height-relaxed); }
.stream-diagnostic__footer { display: flex; align-items: center; gap: var(--ui-space-sm); }
.stream-diagnostic__copy { border-color: var(--ui-border-subtle); }
.stream-diagnostic__copy-state { color: var(--ui-page-text-soft); }
.stream-diagnostic__button[data-focused='true'], .stream-diagnostic__row[data-focused='true'] { color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); position: relative; z-index: 1; }
.stream-diagnostic__row[data-focused='true'] { background: var(--ui-material-selected); }
.stream-diagnostic__row[data-focused='true'] .stream-diagnostic__label, .stream-diagnostic__row[data-focused='true'] .stream-diagnostic__value { color: var(--ui-focus-text); }
.stream-diagnostic-enter-active, .stream-diagnostic-leave-active { transition: opacity var(--ui-motion-fast), transform var(--ui-motion-fast); }
.stream-diagnostic-enter-from, .stream-diagnostic-leave-to { opacity: 0; transform: translateX(var(--ui-space-xl)); }
</style>
