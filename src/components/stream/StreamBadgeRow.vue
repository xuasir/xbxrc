<script setup lang="ts">
import type {
  StreamEnhancementMountState,
  StreamSessionDiagnosticsSnapshot,
} from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

interface StreamBadgeRowProps {
  diagnostics: StreamSessionDiagnosticsSnapshot
  mount: StreamEnhancementMountState
}

interface StreamBadgeViewModel {
  id: 'region' | 'relay' | 'path' | 'server'
  label: string
  value: string
}

const props = defineProps<StreamBadgeRowProps>()

const { t } = useI18n()

const badges = computed<StreamBadgeViewModel[]>(() => {
  const items: StreamBadgeViewModel[] = []

  if (props.diagnostics.regionName !== undefined) {
    items.push({
      id: 'region',
      label: t('streamPage.badges.region'),
      value: props.diagnostics.regionName,
    })
  }

  if (props.diagnostics.turnSource !== 'none') {
    items.push({
      id: 'relay',
      label: t('streamPage.badges.relay'),
      value: t(`streamPage.badges.turnSources.${props.diagnostics.turnSource}`),
    })
  }

  if (props.diagnostics.transportPath !== undefined) {
    items.push({
      id: 'path',
      label: t('streamPage.badges.path'),
      value: props.diagnostics.transportPath,
    })
  }

  if (props.diagnostics.serverHost !== undefined) {
    items.push({
      id: 'server',
      label: t('streamPage.badges.server'),
      value: props.diagnostics.serverHost,
    })
  }

  return items
})

const visible = computed(() =>
  props.mount.phase === 'mounted' && badges.value.length > 0,
)
</script>

<template>
  <div v-if="visible" class="stream-badge-row" :aria-label="t('streamPage.badges.title')">
    <div
      v-for="badge in badges"
      :key="badge.id"
      class="stream-badge-row__item"
    >
      <span class="stream-badge-row__label">{{ badge.label }}</span>
      <strong class="stream-badge-row__value">{{ badge.value }}</strong>
    </div>
  </div>
</template>

<style scoped>
.stream-badge-row {
  position: absolute;
  top: 88px;
  right: 24px;
  z-index: 12;
  max-width: min(520px, calc(100vw - 48px));
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
  pointer-events: none;
}

.stream-badge-row__item {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-height: 32px;
  padding: 6px 10px;
  background: var(--ui-scrim-bg);
  border: 1px solid var(--ui-border-subtle);
  border-radius: 999px;
  color: var(--ui-page-text);
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.24);
}

.stream-badge-row__label {
  font-size: 11px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
}

.stream-badge-row__value {
  font-size: 12px;
  font-weight: 700;
  color: var(--ui-page-text);
  white-space: nowrap;
}

@media (max-width: 768px) {
  .stream-badge-row {
    top: 80px;
    left: 16px;
    right: 16px;
    max-width: none;
    justify-content: flex-start;
  }

  .stream-badge-row__item {
    max-width: 100%;
  }

  .stream-badge-row__value {
    overflow: hidden;
    text-overflow: ellipsis;
  }
}
</style>
