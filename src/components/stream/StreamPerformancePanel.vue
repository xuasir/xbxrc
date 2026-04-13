<script setup lang="ts">
import type { StreamPerformanceSnapshot, StreamSessionDiagnosticsSnapshot } from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

interface StreamPerformancePanelProps {
  visible: boolean
  compact?: boolean
  snapshot: StreamPerformanceSnapshot | null
  diagnostics: StreamSessionDiagnosticsSnapshot
  resolutionMode?: number
  runtimeMode: 'webrtc-direct' | 'rust-owned'
}

const props = withDefaults(defineProps<StreamPerformancePanelProps>(), {
  compact: false,
  resolutionMode: undefined,
})

const { t } = useI18n()
const isBrowserMode = computed(() => props.runtimeMode === 'webrtc-direct')

const resolutionText = computed(() => {
  const resolution = props.snapshot?.resolution
  if (resolution === undefined || resolution === '') {
    return isBrowserMode.value ? '' : '--'
  }

  return props.resolutionMode === 1081 ? `${resolution}(HQ)` : resolution
})

function formatMs(value?: number): string {
  if (value === undefined || Number.isNaN(value)) {
    return '--'
  }

  return `${value.toFixed(1)}ms`
}

function formatKbps(value?: number): string {
  if (value === undefined || value <= 0) {
    return '--'
  }

  if (value >= 1000) {
    return `${(value / 1000).toFixed(1)} Mbps`
  }

  return `${value.toFixed(1)} kbps`
}

function formatFps(value?: string | number): string {
  if (value === undefined || value === null) {
    return '--'
  }
  const numericValue = Number(value)
  if (Number.isNaN(numericValue)) {
    return '--'
  }
  return numericValue.toFixed(1)
}

const metrics = computed(() => [
  { key: 'State', value: resolveStatusText() },
  { key: 'RTT', value: props.snapshot?.rtt ?? '--' },
  { key: 'JIT', value: props.snapshot?.jit ?? '--' },
  { key: 'RecvFPS', value: formatFps(props.snapshot?.inboundVideoFps) },
  { key: 'DecFPS', value: formatFps(props.snapshot?.decodeFps) },
  { key: 'PreFPS', value: formatFps(props.snapshot?.presentFps ?? props.snapshot?.fps) },
  { key: 'PL', value: props.snapshot?.pl ?? '--' },
  { key: 'VideoDL', value: formatKbps(props.snapshot?.inboundVideoBitrateKbps) },
  { key: 'TotalDL', value: formatKbps(props.snapshot?.inboundBitrateKbps) },
  { key: 'AudioDL', value: formatKbps(props.snapshot?.inboundAudioBitrateKbps) },
  { key: 'PktAge', value: formatMs(props.snapshot?.packetAgeMs) },
  { key: 'DecAge', value: formatMs(props.snapshot?.decodeAgeMs) },
  { key: 'PreAge', value: formatMs(props.snapshot?.presentAgeMs) },
  { key: 'P2D', value: formatMs(props.snapshot?.packetToDecodeMs) },
  { key: 'D2P', value: formatMs(props.snapshot?.decodeToPresentMs) },
  { key: 'P2P', value: formatMs(props.snapshot?.packetToPresentMs) },
  { key: 'KeyReq', value: props.snapshot?.recoveryKeyframeRequestCount ?? '--' },
  { key: 'Reset', value: props.snapshot?.videoDecoderResetCount ?? '--' },
  { key: 'DecRec', value: props.snapshot?.videoDecoderRecoveryState ?? '--' },
  { key: 'DecEvt', value: props.snapshot?.videoDecoderRecoveryEvent ?? '--' },
  { key: 'Reco', value: props.snapshot?.recoveryReconnectCount ?? '--' },
].filter((item) => {
  if (!isBrowserMode.value) {
    return true
  }
  if (item.value === '--') {
    return false
  }
  return (
    item.key === 'State'
    || item.key === 'RTT'
    || item.key === 'JIT'
    || item.key === 'PL'
    || item.key === 'RecvFPS'
    || item.key === 'DecFPS'
    || item.key === 'PreFPS'
  )
}))

function resolveStatusText(): string {
  if (props.diagnostics.statusCode === 'noVideo') {
    return t('streamPage.diagnostics.values.noVideo')
  }
  if (props.diagnostics.statusCode === 'probing') {
    return t('streamPage.diagnostics.values.probing')
  }
  if (props.diagnostics.statusCode === 'recovering') {
    return t('streamPage.diagnostics.values.recovering')
  }
  if (props.diagnostics.statusCode === 'blocked') {
    return t('streamPage.diagnostics.values.blocked')
  }
  if (props.diagnostics.statusCode === 'owner' && props.diagnostics.recoveryOwnerState !== undefined) {
    return props.diagnostics.recoveryOwnerState
  }
  if (props.diagnostics.statusCode === 'stable') {
    if (props.diagnostics.isDisplaySupplyLimited) {
      return t('streamPage.diagnostics.values.displaySupplyLimited')
    }
    return t('streamPage.diagnostics.values.stable')
  }
  return t('streamPage.diagnostics.values.inactive')
}
</script>

<template>
  <div v-if="props.visible" class="stream-performance" :class="{ 'stream-performance--compact': props.compact }">
    <strong v-if="!props.compact" class="stream-performance__title">
      {{ t('streamPage.performance.title') }}
    </strong>
    <template v-if="props.compact">
      <span v-if="resolutionText !== ''" class="stream-performance__metric">{{ resolutionText }}</span>
      <span v-for="metric in metrics" :key="metric.key" class="stream-performance__metric">
        {{ t(`streamPage.performance.metrics.${metric.key}`) }}: {{ metric.value }}
      </span>
    </template>

    <template v-else>
      <div v-if="resolutionText !== ''" class="stream-performance__row">
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
  background: var(--ui-surface-info-panel);
  border: 1px solid var(--ui-border-subtle);
  border-radius: 14px;
  color: var(--ui-page-text);
  font-family: var(--ui-font-family-mono, monospace);
  font-size: 11px;
  /* 需要允许滚动，否则数据超出视口会被裁掉且无法滚动查看 */
  pointer-events: auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: min(420px, calc(100vw - 48px));
  max-height: calc(100vh - 48px);
  overflow-y: auto;
  overscroll-behavior: contain;
}

.stream-performance--compact {
  flex-direction: row;
  gap: 12px;
  width: min(1000px, calc(100vw - 48px));
  max-height: unset;
  overflow-y: visible;
  overflow-x: auto;
}

.stream-performance__title {
  margin-bottom: 4px;
  font-size: 12px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
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
