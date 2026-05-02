<script setup lang="ts">
import type {
  StreamEnhancementMountState,
  StreamSessionDiagnosticsSnapshot,
} from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  translateDiagnosticsDecoderRecovery,
  translateDiagnosticsLatestDecision,
  translateDiagnosticsOwnerReason,
  translateDiagnosticsOwnerState,
  translateDiagnosticsPrimaryIssueChain,
  translateDiagnosticsSessionPhase,
  translateDiagnosticsStallKind,
  translateDiagnosticsVideoHealth,
} from '../../streaming/diagnostics-i18n'

interface StreamDiagnosticsPanelProps {
  visible: boolean
  mount: StreamEnhancementMountState
  diagnostics: StreamSessionDiagnosticsSnapshot
  runtimeMode: 'webrtc-direct' | 'rust-owned'
}

interface StreamDiagnosticsRowViewModel {
  key:
    | 'region'
    | 'server'
    | 'relay'
    | 'path'
    | 'inputPortrait'
    | 'phase'
    | 'transportState'
    | 'presentationMilestone'
    | 'connectedElapsed'
    | 'mediaReadyElapsed'
    | 'bandwidthState'
    | 'bandwidthAction'
    | 'recoveryEpoch'
    | 'recoveryLevel'
    | 'recoveryResult'
    | 'recoverySuppressedBy'
    | 'recoveryBudget'
    | 'controlChannelState'
    | 'controlChannelError'
    | 'keyframeSuccessRate'
    | 'networkConfidence'
    | 'decodeConfidence'
    | 'recoveryCause'
    | 'qualityLadderLevel'
    | 'decisionDigest'
    | 'actionEffect'
    | 'actionEffectScore'
    | 'actionEffectReason'
    | 'lastRecoveryReason'
    | 'videoHealth'
    | 'primaryIssueChain'
    | 'latestDecision'
    | 'ownerState'
    | 'ownerReason'
    | 'rfcFaultDomain'
    | 'rfcStage'
    | 'rfcCeiling'
    | 'recoveryDiagnosis'
    | 'decoderState'
    | 'stallKind'
    | 'status'
  value: string
}

interface StreamDiagnosticsNoticeViewModel {
  id: 'probing' | 'recovering' | 'blocked' | 'displaySupply' | 'relayPath' | 'noVideo'
  severity: 'info' | 'warning'
  text: string
}

const props = defineProps<StreamDiagnosticsPanelProps>()

const { t, te } = useI18n()
const isBrowserMode = computed(() => props.runtimeMode === 'webrtc-direct')

const panelVisible = computed(() =>
  props.visible && props.mount.phase === 'mounted',
)

const rows = computed<StreamDiagnosticsRowViewModel[]>(() => {
  const browserMode = isBrowserMode.value
  const items: StreamDiagnosticsRowViewModel[] = []

  const pushIf = (
    key: StreamDiagnosticsRowViewModel['key'],
    value: string | undefined,
    required = false,
  ): void => {
    const normalized = value?.trim()
    if (!required && browserMode && (normalized === undefined || normalized === '')) {
      return
    }
    if (normalized === undefined || normalized === '') {
      return
    }
    items.push({ key, value: normalized })
  }

  pushIf('region', props.diagnostics.regionName ?? t('streamPage.diagnostics.values.unknown'), true)
  pushIf('server', props.diagnostics.serverHost ?? t('streamPage.diagnostics.values.unknown'), true)
  pushIf(
    'relay',
    props.diagnostics.turnSource === 'none'
      ? t('streamPage.diagnostics.values.none')
      : t(`streamPage.badges.turnSources.${props.diagnostics.turnSource}`),
    true,
  )
  pushIf('path', props.diagnostics.transportSummary ?? t('streamPage.diagnostics.values.unknown'), true)
  if (!browserMode) {
    pushIf('inputPortrait', props.diagnostics.recoveryInputPortrait ?? t('streamPage.diagnostics.values.unknown'), true)
  }
  const phaseValue = browserMode && (props.diagnostics.sessionPhase?.trim() ?? '') === ''
    ? undefined
    : translateDiagnosticsSessionPhase(te, t, props.diagnostics.sessionPhase)
  pushIf('phase', phaseValue, !browserMode)
  if (browserMode) {
    pushIf('transportState', props.diagnostics.transportState ?? t('streamPage.diagnostics.values.unknown'), true)
    pushIf('presentationMilestone', props.diagnostics.presentationMilestone ?? t('streamPage.diagnostics.values.unknown'), true)
    pushIf('connectedElapsed', props.diagnostics.connectedMilestoneElapsedText)
    pushIf('mediaReadyElapsed', props.diagnostics.mediaReadyMilestoneElapsedText)
    pushIf('bandwidthState', props.diagnostics.bandwidthState ?? t('streamPage.diagnostics.values.unknown'))
    pushIf('bandwidthAction', props.diagnostics.bandwidthAction ?? t('streamPage.diagnostics.values.none'))
    pushIf('recoveryEpoch', props.diagnostics.recoveryEpochId)
    pushIf('recoveryLevel', props.diagnostics.lastRecoveryActionLevel)
    pushIf('recoveryResult', props.diagnostics.lastRecoveryActionResult)
    pushIf('recoverySuppressedBy', props.diagnostics.recoverySuppressedBy)
    pushIf('recoveryBudget', props.diagnostics.recoveryBudgetRemaining)
    pushIf('controlChannelState', props.diagnostics.controlChannelState)
    pushIf('controlChannelError', props.diagnostics.lastControlChannelError)
    pushIf(
      'keyframeSuccessRate',
      props.diagnostics.keyframeRequestSuccessRate === undefined
        ? undefined
        : `${Math.round(props.diagnostics.keyframeRequestSuccessRate * 100)}%`,
    )
    pushIf('networkConfidence', props.diagnostics.networkConfidence)
    pushIf('decodeConfidence', props.diagnostics.decodeConfidence)
    pushIf('recoveryCause', props.diagnostics.recoveryCause)
    pushIf('qualityLadderLevel', props.diagnostics.qualityLadderLevel)
    pushIf('decisionDigest', props.diagnostics.decisionDigest)
    pushIf('actionEffect', props.diagnostics.lastRecoveryActionEffect)
    pushIf(
      'actionEffectScore',
      props.diagnostics.lastRecoveryActionEffectScore === undefined
        ? undefined
        : props.diagnostics.lastRecoveryActionEffectScore.toFixed(2),
    )
    pushIf('actionEffectReason', props.diagnostics.lastRecoveryActionEffectReason)
    pushIf('lastRecoveryReason', props.diagnostics.lastRecoveryReason ?? t('streamPage.diagnostics.values.none'))
  }
  if (!browserMode) {
    pushIf('videoHealth', translateDiagnosticsVideoHealth(te, t, props.diagnostics.videoHealth), true)
    pushIf('primaryIssueChain', translateDiagnosticsPrimaryIssueChain(te, t, props.diagnostics.primaryIssueChain), true)
    pushIf('latestDecision', translateDiagnosticsLatestDecision(te, t, props.diagnostics.latestDecisionSummary), true)
    pushIf('decoderState', translateDiagnosticsDecoderRecovery(te, t, props.diagnostics.videoDecoderRecoveryState), true)
    pushIf('ownerState', translateDiagnosticsOwnerState(te, t, props.diagnostics.recoveryOwnerState), true)
    pushIf('ownerReason', translateDiagnosticsOwnerReason(te, t, props.diagnostics.recoveryOwnerReason), true)
    pushIf('rfcFaultDomain', props.diagnostics.recoveryRfcFaultDomain?.trim() || t('streamPage.diagnostics.values.unknown'), true)
    pushIf('rfcStage', props.diagnostics.recoveryRfcStage?.trim() || t('streamPage.diagnostics.values.unknown'), true)
    pushIf('rfcCeiling', props.diagnostics.recoveryRfcCeiling?.trim() || t('streamPage.diagnostics.values.unknown'), true)
    pushIf('recoveryDiagnosis', props.diagnostics.recoveryDiagnosis?.trim() || t('streamPage.diagnostics.values.unknown'), true)
    pushIf('stallKind', translateDiagnosticsStallKind(te, t, props.diagnostics.stallKind), true)
  }
  pushIf('status', resolveStatusText(), true)

  return items
})

const notices = computed<StreamDiagnosticsNoticeViewModel[]>(() => {
  const items: StreamDiagnosticsNoticeViewModel[] = []
  const browserMode = isBrowserMode.value

  if (!browserMode) {
    if (props.diagnostics.statusCode === 'probing') {
      items.push({
        id: 'probing',
        severity: 'info',
        text: t('streamPage.diagnostics.notices.probing'),
      })
    }
    else if (props.diagnostics.statusCode === 'blocked') {
      items.push({
        id: 'blocked',
        severity: 'warning',
        text: t('streamPage.diagnostics.notices.blocked'),
      })
    }
    else if (props.diagnostics.isRecovering) {
      items.push({
        id: 'recovering',
        severity: 'info',
        text: t('streamPage.diagnostics.notices.recovering'),
      })
    }
    else if (props.diagnostics.isDisplaySupplyLimited) {
      items.push({
        id: 'displaySupply',
        severity: 'info',
        text: t('streamPage.diagnostics.notices.displaySupplyLimited'),
      })
    }
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
