<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import { useDiagnosticExport } from '../../composables/useDiagnosticExport'

const props = defineProps<{ id: string, scopeId: string, issueId?: string }>()
const { t } = useI18n()
const exporting = useDiagnosticExport()
</script>

<template>
  <span class="diagnostic-export">
    <Focusable :id="props.id" as="button" type="button" :scope-id="props.scopeId" class="ui-action-button" :busy="exporting.busy.value" @click="exporting.save(props.issueId ? { issueId: props.issueId } : {})">
      {{ t(exporting.busy.value ? 'diagnostics.exporting' : 'diagnostics.export') }}
    </Focusable>
    <span v-if="exporting.feedback.value" class="diagnostic-export__feedback" :role="exporting.failed.value ? 'alert' : 'status'">
      {{ exporting.feedback.value }}
    </span>
  </span>
</template>

<style scoped>
.diagnostic-export { display: grid; gap: var(--ui-space-sm); justify-items: start; }
.diagnostic-export .ui-action-button { min-height: 40px; padding: var(--ui-space-sm) var(--ui-space-lg); }
.diagnostic-export__feedback { margin: 0; color: var(--color-text-secondary); font-size: var(--ui-text-body-sm); overflow-wrap: anywhere; }
</style>
