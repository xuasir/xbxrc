<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingModalShell from './SettingModalShell.vue'

const props = defineProps<{ open: boolean, scopeId: string, title: string, message: string, confirmLabel?: string, busy?: boolean }>()
const emit = defineEmits<{ close: [], confirm: [] }>()
const { t } = useI18n()
</script>

<template>
  <SettingModalShell :open="props.open" :scope-id="props.scopeId" :title="props.title" :busy="props.busy" :default-focus-id="`${props.scopeId}.cancel`" @close="emit('close')">
    <p class="setting-panel__notice">
      {{ props.message }}
    </p>
    <template #footer>
      <div class="setting-panel__actions">
        <Focusable :id="`${props.scopeId}.cancel`" as="button" :scope-id="props.scopeId" class="setting-panel__action" :disabled="props.busy" @click="emit('close')">
          {{ t('setting.editor.cancel') }}
        </Focusable>
        <Focusable :id="`${props.scopeId}.confirm`" as="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__action--primary" :disabled="props.busy" @click="emit('confirm')">
          {{ props.confirmLabel || t('setting.experience.confirm') }}
        </Focusable>
      </div>
    </template>
  </SettingModalShell>
</template>

<style scoped src="./setting-layout.css"></style>
