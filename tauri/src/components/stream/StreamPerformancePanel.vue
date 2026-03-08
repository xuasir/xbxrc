<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { StreamPerformanceSnapshot } from '../../streaming/types'

interface StreamPerformancePanelProps {
  visible: boolean
  compact?: boolean
  snapshot: StreamPerformanceSnapshot | null
  resolutionMode?: number
}

const props = withDefaults(defineProps<StreamPerformancePanelProps>(), {
  compact: false,
  resolutionMode: undefined
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
  { key: 'RTT', value: props.snapshot?.rtt ?? '--' },
  { key: 'JIT', value: props.snapshot?.jit ?? '--' },
  { key: 'FPS', value: props.snapshot?.fps ?? '--' },
  { key: 'FD', value: props.snapshot?.fl ?? '--' },
  { key: 'PL', value: props.snapshot?.pl ?? '--' },
  { key: 'Bitrate', value: props.snapshot?.br ?? '--' },
  { key: 'DT', value: props.snapshot?.decode ?? '--' }
])
</script>

<template>
  <div v-if="props.visible" class="stream-performance" :class="{ 'stream-performance--compact': props.compact }">
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
  top: var(--ui-stream-performance-top);
  left: var(--ui-stream-performance-left);
  z-index: 2;
  min-width: var(--ui-stream-performance-min-width);
  padding: var(--ui-stream-performance-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.05), rgba(255, 255, 255, 0.01)),
    rgba(9, 16, 12, 0.76);
  color: #fff;
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.04),
    0 14px 28px rgba(0, 0, 0, 0.22);
  backdrop-filter: blur(14px);
}

.stream-performance--compact {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  min-width: 0;
  max-width: calc(100vw - 48px);
}

.stream-performance__metric {
  font-size: 12px;
  white-space: nowrap;
  color: rgba(255, 255, 255, 0.84);
}

.stream-performance__row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  font-size: 12px;
}

.stream-performance__row + .stream-performance__row {
  margin-top: 6px;
}

.stream-performance__row strong {
  color: rgba(255, 255, 255, 0.96);
}
</style>
