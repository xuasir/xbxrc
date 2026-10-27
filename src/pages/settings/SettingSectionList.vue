<script setup lang="ts">
import type { SettingIndexedRow, SettingIndexedSection, SettingSectionEntry } from './setting-types'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingRow from '../../components/settings/SettingRow.vue'
import SettingToggleRow from '../../components/settings/SettingToggleRow.vue'

const props = defineProps<{
  sections: SettingIndexedSection[]
  scopeId: string
  pendingActionKey: string | null
  expertResetPending: boolean
}>()

const emit = defineEmits<{
  rowConfirm: [row: SettingIndexedRow]
  actionClick: [actionId: string]
}>()

const { t } = useI18n()

function fieldRow(entry: SettingSectionEntry): entry is SettingSectionEntry & { kind: 'field' } {
  return entry.kind === 'field'
}
</script>

<template>
  <div class="setting-panel__list">
    <section
      v-for="section in props.sections"
      :key="section.key"
      class="setting-panel__section"
      :aria-label="section.label"
    >
      <header class="setting-panel__section-header">
        <h2 class="setting-panel__section-title">
          {{ section.label }}
        </h2>
      </header>

      <div class="setting-panel__section-body">
        <template v-for="(entry, ei) in section.entries" :key="`${section.key}-${ei}`">
          <SettingToggleRow
            v-if="fieldRow(entry) && entry.row.control === 'toggle'"
            :id="entry.row.nodeId"
            :scope-id="props.scopeId"
            :label="entry.row.label"
            :enabled="entry.row.value === true"
            :order="entry.row.index"
            @confirm="emit('rowConfirm', entry.row)"
          />

          <SettingRow
            v-else-if="fieldRow(entry)"
            :id="entry.row.nodeId"
            :scope-id="props.scopeId"
            :label="entry.row.label"
            :description="entry.row.description"
            :value="entry.row.valueText"
            :needs-restart="entry.row.needsRestart"
            :select="entry.row.control === 'singleSelect'"
            @click="emit('rowConfirm', entry.row)"
          />

          <Focusable
            v-else-if="entry.kind === 'action'"
            :id="entry.nodeId"
            as="button"
            type="button"
            class="setting-panel__action"
            :class="{
              'setting-panel__action--danger': entry.variant === 'danger',
            }"
            :scope-id="props.scopeId"
            :aria-label="entry.label"
            :disabled="props.pendingActionKey !== null"
            @click="emit('actionClick', entry.actionId)"
          >
            {{
              entry.actionId === 'expertReset' && props.expertResetPending
                ? t('setting.streaming.expert.resetting')
                : entry.label
            }}
          </Focusable>

          <p
            v-else-if="entry.kind === 'notice'"
            class="setting-panel__notice"
          >
            {{ entry.body }}
          </p>

          <div
            v-else-if="entry.kind === 'groupSummary'"
            class="setting-panel__summary"
          >
            <slot :name="`summary-${entry.summaryId}`" />
          </div>
        </template>
      </div>
    </section>
  </div>
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>
