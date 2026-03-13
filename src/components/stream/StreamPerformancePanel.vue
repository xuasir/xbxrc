<script setup lang="ts">
import type { StreamPerformanceSnapshot } from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

interface StreamPerformancePanelProps {
  visible: boolean
  compact?: boolean
  snapshot: StreamPerformanceSnapshot | null
  resolutionMode?: number
}

const props = withDefaults(defineProps<StreamPerformancePanelProps>(), {
  compact: false,
  resolutionMode: undefined,
})

const { t } = useI18n()

const resolutionText = computed(() => {
  const resolution = props.snapshot?.resolution
  if (resolution === undefined || resolution === '') {
    return '--'
  }

  return props.resolutionMode === 1081 ? `${resolution}(HQ)` : resolution
})

const metrics = computed(() => [
  { key: 'Path', value: props.snapshot?.transportPath ?? '--' },
  { key: 'RTT', value: props.snapshot?.rtt ?? '--' },
  { key: 'JIT', value: props.snapshot?.jit ?? '--' },
  { key: 'FPS', value: props.snapshot?.fps ?? '--' },
  { key: 'FD', value: props.snapshot?.fl ?? '--' },
  { key: 'PL', value: props.snapshot?.pl ?? '--' },
  { key: 'Bitrate', value: props.snapshot?.br ?? '--' },
  { key: 'DT', value: props.snapshot?.decode ?? '--' },
])
</script>

<template>
  <div v-if="props.visible" class="stream-performance" :class="{ 'stream-performance--compact': props.compact }">
    <strong v-if="!props.compact" class="stream-performance__title">
      {{ t('streamPage.performance.title') }}
    </strong>
    <template v-if="props.compact">
      <span class="stream-performance__metric">{{ resolutionText }}</span>
      <span v-for="metric in metrics" :key="metric.key" class="stream-performance__metric">
        {{ t(`streamPage.performance.metrics.${metric.key}`) }}: {{ metric.value }}
      </span>
    </template>

    <template v-else>
      <div class="stream-performance__row">
        <span>{{ t('streamPage.performance.metrics.Resolution') }}</span>
        <strong>{{ resolutionText }}</strong>
      </div>
      <div v-for="metric in metrics" :key="metric.key" class="stream-performance__row">
        <span>{{ t(`streamPage.performance.metrics.${metric.key}`) }}</span>
        <strong>{{ metric.value }}</strong>
      </div>
    </template>
  </div>
</template>

<style scoped>
.stream-performance {
  position: absolute;
  top: 24px;
  right: 24px;
  z-index: 14;
  padding: 12px 14px;
  background: rgba(8, 12, 18, 0.84);
  backdrop-filter: blur(14px);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 14px;
  color: #ffffff;
  font-family: var(--ui-font-family-mono, monospace);
  font-size: 11px;
  pointer-events: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.36);
}

.stream-performance--compact {
  flex-direction: row;
  gap: 12px;
}

.stream-performance__title {
  margin-bottom: 4px;
  font-size: 12px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: rgba(255, 255, 255, 0.72);
}

.stream-performance__metric {
  white-space: nowrap;
  opacity: 0.9;
}

.stream-performance__row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.stream-performance__row strong {
  color: var(--brand-accent);
}
</style>
