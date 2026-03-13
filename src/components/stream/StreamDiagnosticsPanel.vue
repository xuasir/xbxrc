<script setup lang="ts">
import type {
  StreamEnhancementMountState,
  StreamSessionDiagnosticsSnapshot,
} from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

interface StreamDiagnosticsPanelProps {
  visible: boolean
  mount: StreamEnhancementMountState
  diagnostics: StreamSessionDiagnosticsSnapshot
}

interface StreamDiagnosticsRowViewModel {
  key: 'region' | 'server' | 'relay' | 'path' | 'status'
  value: string
}

interface StreamDiagnosticsNoticeViewModel {
  id: 'recovering' | 'relayPath' | 'noVideo'
  severity: 'info' | 'warning'
  text: string
}

const props = defineProps<StreamDiagnosticsPanelProps>()

const { t } = useI18n()

const panelVisible = computed(() =>
  props.visible && props.mount.phase === 'mounted',
)

const rows = computed<StreamDiagnosticsRowViewModel[]>(() => [
  {
    key: 'region',
    value: props.diagnostics.regionName ?? t('streamPage.diagnostics.values.unknown'),
  },
  {
    key: 'server',
    value: props.diagnostics.serverHost ?? t('streamPage.diagnostics.values.unknown'),
  },
  {
    key: 'relay',
    value:
      props.diagnostics.turnSource === 'none'
        ? t('streamPage.diagnostics.values.none')
        : t(`streamPage.badges.turnSources.${props.diagnostics.turnSource}`),
  },
  {
    key: 'path',
    value: props.diagnostics.transportPath ?? t('streamPage.diagnostics.values.unknown'),
  },
  {
    key: 'status',
    value: resolveStatusText(),
  },
])

const notices = computed<StreamDiagnosticsNoticeViewModel[]>(() => {
  const items: StreamDiagnosticsNoticeViewModel[] = []

  if (props.diagnostics.isRecovering) {
    items.push({
      id: 'recovering',
      severity: 'info',
      text: t('streamPage.diagnostics.notices.recovering'),
    })
  }

  if (props.diagnostics.isRelayPath) {
    items.push({
      id: 'relayPath',
      severity: 'info',
      text: t('streamPage.diagnostics.notices.relayPath'),
    })
  }

  if (props.diagnostics.hasNoVideoWarning) {
    items.push({
      id: 'noVideo',
      severity: 'warning',
      text: t('streamPage.diagnostics.notices.noVideo'),
    })
  }

  return items
})

function resolveStatusText(): string {
  if (props.diagnostics.hasNoVideoWarning) {
    return t('streamPage.diagnostics.values.noVideo')
  }
  if (props.diagnostics.isRecovering) {
    return t('streamPage.diagnostics.values.recovering')
  }
  if (props.diagnostics.isActive) {
    return t('streamPage.diagnostics.values.stable')
  }
  return t('streamPage.diagnostics.values.inactive')
}
</script>

<template>
  <aside v-if="panelVisible" class="stream-diagnostics-panel" :aria-label="t('streamPage.diagnostics.panel.title')">
    <header class="stream-diagnostics-panel__header">
      <span class="stream-diagnostics-panel__eyebrow">{{ t('streamPage.diagnostics.panel.eyebrow') }}</span>
      <strong class="stream-diagnostics-panel__title">{{ t('streamPage.diagnostics.panel.title') }}</strong>
    </header>

    <div class="stream-diagnostics-panel__rows">
      <div
        v-for="row in rows"
        :key="row.key"
        class="stream-diagnostics-panel__row"
      >
        <span>{{ t(`streamPage.diagnostics.fields.${row.key}`) }}</span>
        <strong>{{ row.value }}</strong>
      </div>
    </div>

    <div v-if="notices.length > 0" class="stream-diagnostics-panel__notices">
      <div
        v-for="notice in notices"
        :key="notice.id"
        class="stream-diagnostics-panel__notice"
        :class="`stream-diagnostics-panel__notice--${notice.severity}`"
      >
        {{ notice.text }}
      </div>
    </div>
  </aside>
</template>

<style scoped>
.stream-diagnostics-panel {
  position: absolute;
  top: 132px;
  right: 24px;
  z-index: 14;
  width: min(340px, calc(100vw - 48px));
  padding: 14px 16px;
  border-radius: 16px;
  background: rgba(8, 12, 18, 0.84);
  border: 1px solid rgba(255, 255, 255, 0.12);
  backdrop-filter: blur(18px);
  color: #ffffff;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.36);
  pointer-events: none;
}

.stream-diagnostics-panel__header {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-bottom: 10px;
}

.stream-diagnostics-panel__eyebrow {
  font-size: 11px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: rgba(255, 255, 255, 0.72);
}

.stream-diagnostics-panel__title {
  font-size: 14px;
  font-weight: 700;
}

.stream-diagnostics-panel__rows {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.stream-diagnostics-panel__row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
  font-size: 12px;
}

.stream-diagnostics-panel__row span {
  color: rgba(255, 255, 255, 0.72);
}

.stream-diagnostics-panel__row strong {
  max-width: 58%;
  text-align: right;
  word-break: break-word;
}

.stream-diagnostics-panel__notices {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 12px;
}

.stream-diagnostics-panel__notice {
  padding: 8px 10px;
  border-radius: 12px;
  font-size: 12px;
  line-height: 1.45;
}

.stream-diagnostics-panel__notice--info {
  background: rgba(255, 255, 255, 0.08);
}

.stream-diagnostics-panel__notice--warning {
  background: rgba(159, 84, 24, 0.36);
  border: 1px solid rgba(255, 184, 77, 0.28);
}

@media (max-width: 768px) {
  .stream-diagnostics-panel {
    top: 148px;
    left: 16px;
    right: 16px;
    width: auto;
  }
}
</style>
