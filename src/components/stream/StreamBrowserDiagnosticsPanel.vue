<script setup lang="ts">
import type { StreamBrowserDiagnosticsViewModel, StreamEnhancementMountState } from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { browserDiagnosticsRows } from '../../streaming/stream-panel-view-models'

const props = defineProps<{
  visible: boolean
  mount: StreamEnhancementMountState
  model: StreamBrowserDiagnosticsViewModel
}>()

const { t } = useI18n()

const panelVisible = computed(() =>
  props.visible && props.mount.phase === 'mounted',
)

const rows = computed(() => browserDiagnosticsRows(props.model))
</script>

<template>
  <aside
    v-if="panelVisible"
    class="stream-browser-diagnostics"
    :aria-label="t('streamPage.diagnosticsPanel.browser.title')"
  >
    <header class="stream-browser-diagnostics__header">
      <span class="stream-browser-diagnostics__eyebrow">{{ t('streamPage.diagnostics.panel.eyebrow') }}</span>
      <strong class="stream-browser-diagnostics__title">{{ t('streamPage.diagnosticsPanel.browser.title') }}</strong>
    </header>
    <div class="stream-browser-diagnostics__rows">
      <div
        v-for="row in rows"
        :key="row.key"
        class="stream-browser-diagnostics__row"
      >
        <span>{{ t(`streamPage.diagnosticsPanel.browser.fields.${row.key}`) }}</span>
        <strong>{{ row.value }}</strong>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.stream-browser-diagnostics {
  position: absolute;
  top: 132px;
  right: 24px;
  z-index: 14;
  width: min(340px, calc(100vw - 48px));
  padding: 14px 16px;
  border-radius: 16px;
  background: var(--ui-surface-info-panel);
  border: 1px solid var(--ui-border-subtle);
  color: var(--ui-page-text);
  pointer-events: auto;
  max-height: calc(100vh - 156px);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.stream-browser-diagnostics__header {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-bottom: 10px;
}

.stream-browser-diagnostics__eyebrow {
  font-size: 11px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
}

.stream-browser-diagnostics__title {
  font-size: 14px;
  font-weight: 700;
}

.stream-browser-diagnostics__rows {
  display: flex;
  flex-direction: column;
  gap: 8px;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding-right: 2px;
  flex: 1;
  min-height: 0;
  font-size: 12px;
}

.stream-browser-diagnostics__row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
}

.stream-browser-diagnostics__row span {
  color: var(--ui-page-text-soft);
}

.stream-browser-diagnostics__row strong {
  max-width: 58%;
  text-align: right;
  word-break: break-word;
  font-family: var(--ui-font-family-mono, monospace);
  font-size: 11px;
}

@media (max-width: 768px) {
  .stream-browser-diagnostics {
    top: 148px;
    left: 16px;
    right: 16px;
    width: auto;
    max-height: calc(100vh - 164px);
  }
}
</style>
