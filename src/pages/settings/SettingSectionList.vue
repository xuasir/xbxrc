<script setup lang="ts">
import type { SettingIndexedRow, SettingIndexedSection } from './setting-types'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingInlineSingleSelect from '../../components/settings/SettingInlineSingleSelect.vue'
import SettingRow from '../../components/settings/SettingRow.vue'
import SettingToggleRow from '../../components/settings/SettingToggleRow.vue'

const props = defineProps<{ sections: SettingIndexedSection[], scopeId: string, pendingActionKey: string | null, expanded: Set<string> }>()
const emit = defineEmits<{ 'rowConfirm': [row: SettingIndexedRow], 'rowSelect': [row: SettingIndexedRow, value: string | number], 'actionClick': [actionId: string], 'update:expanded': [value: Set<string>] }>()
const { t } = useI18n()
function toggle(key: string) {
  const next = new Set(props.expanded)
  if (next.has(key))
    next.delete(key)
  else next.add(key)
  emit('update:expanded', next)
}
function effect(row: SettingIndexedRow) {
  return row.effect && row.effect !== 'instant' ? t(`setting.experience.effects.${row.effect}`) : undefined
}
</script>

<template>
  <div class="setting-panel__list">
    <section v-for="section in props.sections" :key="section.key" class="setting-panel__section" :aria-label="section.label">
      <Focusable v-if="section.collapsible" :id="`${props.scopeId}.section.${section.key}`" as="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__collapse" :aria-expanded="expanded.has(section.key)" :aria-controls="`${props.scopeId}.body.${section.key}`" @click="toggle(section.key)">
        <span>{{ section.label }}</span><span aria-hidden="true">{{ expanded.has(section.key) ? '−' : '+' }}</span>
      </Focusable>
      <header v-else class="setting-panel__section-header">
        <h2 class="setting-panel__section-title">
          {{ section.label }}
        </h2>
      </header>
      <div v-if="!section.collapsible || expanded.has(section.key)" :id="`${props.scopeId}.body.${section.key}`" class="setting-panel__section-body">
        <template v-for="(entry, i) in section.entries" :key="entry.kind === 'field' ? entry.row.key : i">
          <template v-if="entry.kind === 'field'">
            <SettingToggleRow v-if="entry.row.control === 'toggle'" :id="entry.row.nodeId" :scope-id="props.scopeId" :label="entry.row.label" :description="entry.row.description" :effect-label="effect(entry.row)" :enabled="entry.row.value === true" :busy="props.pendingActionKey !== null" @confirm="emit('rowConfirm', entry.row)" />
            <div v-else-if="entry.row.control === 'singleSelect' && entry.row.key !== 'xcloud_region_name' && entry.row.options && entry.row.options.length <= 3" class="setting-panel__inline-field">
              <h3 class="setting-panel__inline-label">
                {{ entry.row.label }}
              </h3>
              <p v-if="entry.row.description" class="setting-panel__status">
                {{ entry.row.description }}
              </p>
              <p v-if="effect(entry.row)" class="setting-panel__effect">
                {{ effect(entry.row) }}
              </p>
              <SettingInlineSingleSelect open :scope-id="props.scopeId" :row-node-id="entry.row.nodeId" :label="entry.row.label" :options="entry.row.options" :current-value="typeof entry.row.value === 'object' ? entry.row.options.find(option => option.label === entry.row.valueText)?.value ?? null : (entry.row.value as string | number)" :busy="props.pendingActionKey !== null" @select="emit('rowSelect', entry.row, $event)" />
            </div>
            <SettingRow v-else :id="entry.row.nodeId" :scope-id="props.scopeId" :label="entry.row.label" :description="entry.row.description" :value="entry.row.valueText" :effect-label="effect(entry.row)" :busy="props.pendingActionKey !== null" select @click="emit('rowConfirm', entry.row)" />
          </template>
          <SettingRow v-else-if="entry.kind === 'action'" :id="entry.nodeId" :scope-id="props.scopeId" :label="entry.label" :description="entry.description" :value="entry.value" :busy="props.pendingActionKey !== null" select @click="emit('actionClick', entry.actionId)" />
          <p v-else-if="entry.kind === 'notice'" class="setting-panel__notice">
            {{ entry.body }}
          </p>
        </template>
      </div>
    </section>
  </div>
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>

<style scoped>
.setting-panel__inline-field { padding: var(--ui-space-md) 0 0; border-radius: var(--ui-radius-md); background: var(--ui-settings-item-bg); }
.setting-panel__inline-label { margin: 0 0 var(--ui-space-xs); padding: 0 var(--ui-settings-control-padding); font-size: var(--ui-settings-row-label-size); color: var(--color-text-primary); font-weight: var(--ui-font-weight-medium); }
.setting-panel__inline-field > .setting-panel__status { padding: 0 var(--ui-settings-control-padding); font-size: var(--ui-settings-row-description-size); }
</style>
