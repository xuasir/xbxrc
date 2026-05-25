<script setup lang="ts">
import type { StreamEnhancementMountState, StreamExperienceMetricsViewModel } from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { EXPERIENCE_METRIC_KEYS, experienceMetricValue } from '../../streaming/stream-panel-view-models'

const props = defineProps<{
  visible: boolean
  compact: boolean
  mount: StreamEnhancementMountState
  model: StreamExperienceMetricsViewModel
}>()

const { t } = useI18n()

const panelVisible = computed(() =>
  props.visible && props.mount.phase === 'mounted',
)

const metricRows = computed(() => {
  const vm = props.model
  return EXPERIENCE_METRIC_KEYS.flatMap((key) => {
    const value = experienceMetricValue(vm, key)
    if (key === 'resolution' && value === '') {
      return []
    }
    if ((key === 'connectedElapsed' || key === 'mediaReadyElapsed') && value === '--') {
      return []
    }
    return [{ key, value }]
  })
})

const notices = computed(() => {
  const vm = props.model
  const items: Array<{ id: string, text: string }> = []
  if (vm.relayNotice) {
    items.push({ id: 'relay', text: t('streamPage.experience.notices.relay') })
  }
  if (vm.displaySupplyNotice) {
    items.push({ id: 'displaySupply', text: t('streamPage.experience.notices.displaySupplyLimited') })
  }
  else if (vm.recoveringNotice) {
    items.push({ id: 'recovering', text: t('streamPage.experience.notices.recovering') })
  }
  if (vm.noVideoNotice) {
    items.push({ id: 'noVideo', text: t('streamPage.experience.notices.noVideo') })
  }
  return items
})
</script>

<template>
  <div
    v-if="panelVisible"
    class="stream-experience"
    :class="{ 'stream-experience--compact': compact }"
  >
    <strong v-if="!compact" class="stream-experience__title">
      {{ t('streamPage.experience.title') }}
    </strong>
    <template v-if="compact">
      <span
        v-for="row in metricRows"
        :key="row.key"
        class="stream-experience__metric"
      >
        {{ t(`streamPage.experience.metrics.${row.key}`) }}: {{ row.value }}
      </span>
    </template>
    <template v-else>
      <div
        v-for="row in metricRows"
        :key="row.key"
        class="stream-experience__row"
      >
        <span>{{ t(`streamPage.experience.metrics.${row.key}`) }}</span>
        <strong>{{ row.value }}</strong>
      </div>
    </template>
    <div v-if="notices.length > 0" class="stream-experience__notices">
      <div
        v-for="n in notices"
        :key="n.id"
        class="stream-experience__notice"
      >
        {{ n.text }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.stream-experience {
  position: absolute;
  top: 24px;
  right: 24px;
  z-index: 14;
  padding: 12px 14px;
  background: var(--ui-surface-info-panel);
  border: 1px solid var(--ui-border-subtle);
  border-radius: 14px;
  color: var(--ui-page-text);
  font-family: var(--ui-font-family-mono, monospace);
  font-size: 11px;
  pointer-events: auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: min(420px, calc(100vw - 48px));
  max-height: calc(100vh - 48px);
  overflow-y: auto;
  overscroll-behavior: contain;
}

.stream-experience--compact {
  flex-direction: row;
  flex-wrap: wrap;
  gap: 12px;
  width: min(1000px, calc(100vw - 48px));
  max-height: unset;
  overflow-y: visible;
  overflow-x: auto;
}

.stream-experience__title {
  margin-bottom: 4px;
  font-size: 12px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
}

.stream-experience__metric {
  white-space: nowrap;
  opacity: 0.9;
}

.stream-experience__row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.stream-experience__row strong {
  color: var(--brand-accent);
}

.stream-experience__notices {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--ui-border-subtle);
}

.stream-experience__notice {
  font-size: 11px;
  line-height: 1.4;
  color: var(--ui-page-text-soft);
}
</style>
