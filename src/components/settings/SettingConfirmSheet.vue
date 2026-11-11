<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import ExportDiagnosticsButton from '../common/ExportDiagnosticsButton.vue'
import SettingModalShell from './SettingModalShell.vue'

const props = defineProps<{ open: boolean, scopeId: string, title: string, message: string, confirmLabel?: string, eyebrow?: string, diagnosticIssueId?: string, busy?: boolean }>()
const emit = defineEmits<{ close: [], confirm: [] }>()
const { t } = useI18n()
</script>

<template>
  <SettingModalShell :open="props.open" :scope-id="props.scopeId" :eyebrow="props.eyebrow" :title="props.title" :busy="props.busy" :default-focus-id="`${props.scopeId}.cancel`" @close="emit('close')">
    <p class="setting-panel__notice">
      {{ props.message }}
    </p>
    <ExportDiagnosticsButton v-if="props.diagnosticIssueId" :id="`${props.scopeId}.export`" :scope-id="props.scopeId" :issue-id="props.diagnosticIssueId" />
    <template #footer>
      <div class="setting-panel__actions">
        <Focusable :id="`${props.scopeId}.cancel`" as="button" :scope-id="props.scopeId" class="setting-panel__action" :busy="props.busy" @click="emit('close')">
          {{ t('setting.editor.cancel') }}
        </Focusable>
        <Focusable :id="`${props.scopeId}.confirm`" as="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__action--primary" :busy="props.busy" @click="emit('confirm')">
          {{ props.confirmLabel || t('setting.experience.confirm') }}
        </Focusable>
      </div>
    </template>
  </SettingModalShell>
</template>

<style scoped src="./setting-layout.css"></style>
