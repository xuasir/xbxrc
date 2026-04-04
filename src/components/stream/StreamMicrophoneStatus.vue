<script setup lang="ts">
import type { StreamEnhancementMountState, StreamMicrophoneSnapshot } from '../../streaming/types'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

interface StreamMicrophoneStatusProps {
  mount: StreamEnhancementMountState
  microphone: StreamMicrophoneSnapshot
}

const props = defineProps<StreamMicrophoneStatusProps>()

const { t } = useI18n()

const visible = computed(() => props.mount.phase === 'mounted')
const label = computed(() =>
  props.microphone.phase === 'live'
    ? t('streamPage.microphone.open')
    : props.microphone.phase === 'starting'
      ? t('streamPage.microphone.starting')
      : props.microphone.phase === 'paused'
        ? t('streamPage.microphone.paused')
        : t('streamPage.microphone.closed'),
)
const sourceLabel = computed(() =>
  props.microphone.activationSource === 'policy'
    ? t('streamPage.microphone.sources.policy')
    : props.microphone.activationSource === 'user'
      ? t('streamPage.microphone.sources.user')
      : '',
)
</script>

<template>
  <div v-if="visible" class="stream-microphone-status">
    <span
      class="stream-microphone-status__dot"
      :class="{ 'stream-microphone-status__dot--live': props.microphone.open }"
      aria-hidden="true"
    />
    <span class="stream-microphone-status__label">
      {{ label }}
      <span v-if="sourceLabel !== ''" class="stream-microphone-status__source">
        {{ sourceLabel }}
      </span>
    </span>
  </div>
</template>

<style scoped>
.stream-microphone-status {
  position: absolute;
  right: 24px;
  bottom: 24px;
  z-index: 12;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-height: 32px;
  padding: 6px 12px;
  background: var(--ui-scrim-bg);
  border: 1px solid var(--ui-border-subtle);
  border-radius: 999px;
  color: var(--ui-page-text);
  pointer-events: none;
}

.stream-microphone-status__dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--ui-page-text) 35%, transparent);
}

.stream-microphone-status__dot--live {
  background: var(--color-success);
  box-shadow: 0 0 10px color-mix(in srgb, var(--color-success) 45%, transparent);
}

.stream-microphone-status__label {
  font-size: 12px;
  font-weight: 700;
}

.stream-microphone-status__source {
  margin-left: 6px;
  opacity: 0.7;
}

@media (max-width: 768px) {
  .stream-microphone-status {
    right: 16px;
    bottom: 16px;
  }
}
</style>
