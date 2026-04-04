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
  key:
    | 'region'
    | 'server'
    | 'relay'
    | 'path'
    | 'inputPortrait'
    | 'phase'
    | 'ownerState'
    | 'ownerReason'
    | 'decoderState'
    | 'status'
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
    value: props.diagnostics.transportSummary ?? t('streamPage.diagnostics.values.unknown'),
  },
  {
    key: 'inputPortrait',
    value: props.diagnostics.recoveryInputPortrait ?? t('streamPage.diagnostics.values.unknown'),
  },
  {
    key: 'phase',
    value: props.diagnostics.sessionPhase ?? t('streamPage.diagnostics.values.unknown'),
  },
  {
    key: 'decoderState',
    value:
      props.diagnostics.videoDecoderRecoveryState ?? t('streamPage.diagnostics.values.unknown'),
  },
  {
    key: 'ownerState',
    value: props.diagnostics.recoveryOwnerState ?? t('streamPage.diagnostics.values.unknown'),
  },
  {
    key: 'ownerReason',
    value: props.diagnostics.recoveryOwnerReason ?? t('streamPage.diagnostics.values.none'),
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
  if (props.diagnostics.statusCode === 'noVideo') {
    return t('streamPage.diagnostics.values.noVideo')
  }
  if (props.diagnostics.statusCode === 'recovering') {
    return t('streamPage.diagnostics.values.recovering')
  }
  if (props.diagnostics.statusCode === 'owner' && props.diagnostics.recoveryOwnerState !== undefined) {
    return props.diagnostics.recoveryOwnerState
  }
  if (props.diagnostics.statusCode === 'stable') {
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
  background: var(--ui-surface-info-panel);
  border: 1px solid var(--ui-border-subtle);
  color: var(--ui-page-text);
  /* 需要允许滚动，否则信息较多时超出视口会被裁掉且无法滚动查看 */
  pointer-events: auto;
  max-height: calc(100vh - 156px);
  display: flex;
  flex-direction: column;
  overflow: hidden;
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
  color: var(--ui-page-text-soft);
}

.stream-diagnostics-panel__title {
  font-size: 14px;
  font-weight: 700;
}

.stream-diagnostics-panel__rows {
  display: flex;
  flex-direction: column;
  gap: 8px;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding-right: 2px;
  flex: 1;
  min-height: 0;
}

.stream-diagnostics-panel__row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
  font-size: 12px;
}

.stream-diagnostics-panel__row span {
  color: var(--ui-page-text-soft);
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
  background: var(--color-state-hover);
}

.stream-diagnostics-panel__notice--warning {
  background: var(--ui-notice-warning-bg);
  border: 1px solid var(--ui-notice-warning-border);
}

@media (max-width: 768px) {
  .stream-diagnostics-panel {
    top: 148px;
    left: 16px;
    right: 16px;
    width: auto;
    max-height: calc(100vh - 164px);
  }
}
</style>
