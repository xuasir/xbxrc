<script setup lang="ts">
import type {
  StreamEnhancementMountState,
  StreamSessionDiagnosticsSnapshot,
} from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

interface StreamDiagnosticNoticeBarProps {
  diagnostics: StreamSessionDiagnosticsSnapshot
  mount: StreamEnhancementMountState
}

interface StreamDiagnosticNoticeViewModel {
  id: 'recovering' | 'displaySupply' | 'relayPath' | 'noVideo'
  severity: 'info' | 'warning'
  text: string
}

const props = defineProps<StreamDiagnosticNoticeBarProps>()

const { t } = useI18n()

const notices = computed<StreamDiagnosticNoticeViewModel[]>(() => {
  const items: StreamDiagnosticNoticeViewModel[] = []

  if (props.diagnostics.isRecovering) {
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
</script>

<template>
  <div v-if="mount.phase === 'mounted' && notices.length > 0" class="stream-diagnostic-notice-bar">
    <div
      v-for="notice in notices"
      :key="notice.id"
      class="stream-diagnostic-notice-bar__item"
      :class="`stream-diagnostic-notice-bar__item--${notice.severity}`"
    >
      {{ notice.text }}
    </div>
  </div>
</template>

<style scoped>
.stream-diagnostic-notice-bar {
  position: absolute;
  top: 132px;
  right: 24px;
  z-index: 12;
  max-width: min(560px, calc(100vw - 48px));
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 8px;
  pointer-events: none;
}

.stream-diagnostic-notice-bar__item {
  max-width: 100%;
  padding: 10px 14px;
  border-radius: 14px;
  color: var(--ui-page-text);
  font-size: 12px;
  font-weight: 600;
  line-height: 1.4;
  box-shadow: var(--ui-shadow-floating);
}

.stream-diagnostic-notice-bar__item--info {
  background: var(--ui-scrim-bg);
  border: 1px solid var(--ui-border-subtle);
}

.stream-diagnostic-notice-bar__item--warning {
  background: var(--ui-notice-warning-mix);
  border: 1px solid var(--ui-notice-warning-border);
}

@media (max-width: 768px) {
  .stream-diagnostic-notice-bar {
    top: 172px;
    left: 16px;
    right: 16px;
    max-width: none;
    align-items: stretch;
  }
}
</style>
