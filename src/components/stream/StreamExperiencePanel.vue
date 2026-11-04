<script setup lang="ts">
import type { StreamEnhancementMountState, StreamExperienceMetricsViewModel } from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { experienceMetricValue, RUNTIME_METRIC_KEYS } from '../../streaming/stream-panel-view-models'

const props = defineProps<{
  visible: boolean
  mount: StreamEnhancementMountState
  model: StreamExperienceMetricsViewModel
  suppressNotice?: boolean
}>()
const { t } = useI18n()
const panelVisible = computed(() => props.visible && props.mount.phase === 'mounted')
const rows = computed(() => RUNTIME_METRIC_KEYS.map(key => ({ key, value: experienceMetricValue(props.model, key) })))
const notice = computed(() => !props.suppressNotice && props.model.notice ? t(`streamPage.experience.notices.${props.model.notice}`) : '')
</script>

<template>
  <div v-if="panelVisible" class="stream-runtime-bar" :aria-label="t('streamPage.experience.title')" :data-notice="model.notice">
    <template v-for="row in rows" :key="row.key">
      <span v-if="row.key !== 'videoBitrate' || !notice" class="stream-runtime-bar__metric" :class="`stream-runtime-bar__metric--${row.key}`">
        <span v-if="row.key === 'rtt' || row.key === 'packetLoss'" class="stream-runtime-bar__label">{{ t(`streamPage.experience.metrics.${row.key}`) }}</span>
        <span>{{ row.value }}</span>
        <span v-if="row.key === 'presentFps'" class="stream-runtime-bar__label">FPS</span>
      </span>
      <span v-else class="stream-runtime-bar__metric stream-runtime-bar__notice" aria-live="polite">{{ notice }}</span>
    </template>
  </div>
</template>

<style scoped>
.stream-runtime-bar {
  position: absolute;
  top: var(--ui-stream-metrics-top);
  left: 50%;
  transform: translateX(-50%);
  z-index: var(--z-stream-metrics);
  max-width: calc(100% - var(--ui-stream-metrics-inset) * 2);
  min-height: var(--ui-stream-metrics-height);
  display: flex;
  align-items: center;
  padding: var(--ui-space-xs);
  border-radius: 0 0 var(--ui-radius-sm) var(--ui-radius-sm);
  background: var(--ui-stream-metrics-bg);
  color: var(--ui-stream-metrics-text);
  font-family: var(--ui-font-family);
  font-size: var(--ui-stream-metrics-font-size);
  font-variant-numeric: tabular-nums;
  line-height: var(--ui-line-height-default);
  text-shadow: var(--ui-stream-metrics-text-shadow);
  white-space: nowrap;
  pointer-events: none;
}
.stream-runtime-bar__metric {
  display: inline-flex;
  justify-content: center;
  align-items: baseline;
  gap: var(--ui-space-xs);
  padding-inline: var(--ui-stream-metrics-cell-padding);
  min-width: var(--ui-stream-metrics-cell-width);
}
.stream-runtime-bar__metric + .stream-runtime-bar__metric {
  border-left: 1px solid var(--ui-stream-metrics-divider);
}
.stream-runtime-bar__label { color: var(--ui-stream-metrics-label); }
.stream-runtime-bar__notice { color: var(--color-warning); }
.stream-runtime-bar[data-notice='sampling'] .stream-runtime-bar__notice,
.stream-runtime-bar[data-notice='stale'] .stream-runtime-bar__notice { color: var(--ui-stream-metrics-label); }
.stream-runtime-bar[data-notice='noVideo'] .stream-runtime-bar__notice { color: var(--color-danger); }
</style>
