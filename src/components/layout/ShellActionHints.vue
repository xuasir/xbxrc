<script setup lang="ts">
import { useNavigationButtonLabels } from '@shared/gamepad/navigation-hints'
import { useI18n } from 'vue-i18n'

defineProps<{
  regionLabel?: string
  status?: string
  error?: boolean
  overlay?: boolean
  available?: boolean
  exclusive?: boolean
  busy?: boolean
  back?: boolean
}>()
const { t } = useI18n()
const buttons = useNavigationButtonLabels()
</script>

<template>
  <footer class="shell-hints" :aria-label="t('shell.hints.label')">
    <div v-if="available && !exclusive && !busy" class="shell-hints__actions">
      <span class="shell-hints__action"><kbd>{{ buttons.south }}</kbd>{{ t('shell.hints.confirm') }}</span>
      <span v-if="overlay || back" class="shell-hints__action"><kbd>{{ buttons.east }}</kbd>{{ t(overlay ? 'shell.hints.close' : 'shell.hints.back') }}</span>
      <template v-if="!overlay">
        <span class="shell-hints__action"><kbd>{{ buttons.l1 }}</kbd><kbd>{{ buttons.r1 }}</kbd>{{ t('shell.hints.pages') }}</span>
        <span v-if="regionLabel" class="shell-hints__action"><kbd>{{ buttons.l2 }}</kbd><kbd>{{ buttons.r2 }}</kbd>{{ regionLabel }}</span>
      </template>
    </div>
    <span v-else class="shell-hints__ownership">
      {{ t(busy ? 'shell.hints.busy' : exclusive ? 'shell.hints.exclusive' : 'shell.hints.inactive') }}
    </span>
    <p class="shell-hints__status" :class="{ 'shell-hints__status--error': error }" :role="error ? 'alert' : 'status'">
      {{ status }}
    </p>
  </footer>
</template>

<style scoped>
.shell-hints { grid-column: 2; grid-row: 3; position: relative; z-index: calc(var(--z-modal) + 1); display: flex; align-items: center; justify-content: space-between; gap: var(--space-16); min-width: 0; min-height: var(--ui-shell-hint-area); padding: var(--space-12) var(--space-16) var(--space-16); color: var(--color-text-secondary); background: transparent; pointer-events: none; font-size: 12px; line-height: 18px; }
.shell-hints__actions { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-24); }
.shell-hints__action { display: inline-flex; align-items: center; gap: var(--space-8); white-space: nowrap; }
.shell-hints kbd { display: inline-flex; align-items: center; justify-content: center; min-width: 24px; min-height: 22px; padding: 1px var(--space-4); border: 1px solid var(--ui-border-hover); border-radius: 4px; background: var(--ui-keycap-bg); color: var(--ui-icon-text); font: inherit; font-size: 11px; line-height: 16px; box-shadow: inset 0 1px 0 var(--ui-material-edge); }
.shell-hints__status { min-width: 0; max-width: 42%; overflow-wrap: anywhere; text-align: right; }
.shell-hints__status--error { color: var(--color-danger); }
@media(max-width:700px) { .shell-hints { grid-column: 1; grid-row: 4; padding: var(--space-12) var(--space-4) var(--space-16); flex-wrap: wrap; min-height: 64px; gap: var(--space-8); } .shell-hints__actions { gap: var(--space-12); } .shell-hints__status { max-width: 100%; text-align: left; } }
</style>
