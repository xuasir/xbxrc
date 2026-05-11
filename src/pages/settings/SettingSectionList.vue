<script setup lang="ts">
import type { SettingIndexedRow, SettingIndexedSection, SettingSectionEntry } from './setting-types'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingToggleRow from '../../components/settings/SettingToggleRow.vue'

const props = defineProps<{
  sections: SettingIndexedSection[]
  scopeId: string
  pendingActionKey: string | null
  expertResetPending: boolean
}>()

const emit = defineEmits<{
  rowConfirm: [row: SettingIndexedRow]
  toolClick: [toolId: string]
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

          <Focusable
            v-else-if="fieldRow(entry) && entry.row.control !== 'toggle'"
            :id="entry.row.nodeId"
            as="button"
            type="button"
            class="setting-row"
            :class="{
              'setting-row--select': entry.row.control === 'singleSelect',
              'setting-row--restart': entry.row.needsRestart,
            }"
            :scope-id="props.scopeId"
            :aria-label="entry.row.label"
            @click="emit('rowConfirm', entry.row)"
          >
            <span
              class="setting-row__copy"
              :class="{ 'setting-row__copy--singleline': !entry.row.description }"
            >
              <span class="setting-row__label">
                {{ entry.row.label }}
                <span
                  v-if="entry.row.needsRestart"
                  class="setting-row__badge"
                >{{ t('setting.effects.restartBadge') }}</span>
              </span>
              <span v-if="entry.row.description" class="setting-row__desc">{{ entry.row.description }}</span>
            </span>
            <span class="setting-row__value">{{ entry.row.valueText }}</span>
          </Focusable>

          <Focusable
            v-else-if="entry.kind === 'tool'"
            :id="entry.nodeId"
            as="button"
            type="button"
            class="setting-row setting-row--select"
            :scope-id="props.scopeId"
            :aria-label="entry.label"
            :disabled="props.pendingActionKey !== null"
            @click="emit('toolClick', entry.toolId)"
          >
            <span
              class="setting-row__copy"
              :class="{ 'setting-row__copy--singleline': !entry.description }"
            >
              <span class="setting-row__label">{{ entry.label }}</span>
              <span v-if="entry.description" class="setting-row__desc">{{ entry.description }}</span>
            </span>
            <span class="setting-row__value">{{ entry.valueText }}</span>
          </Focusable>

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

<style scoped>
.setting-panel__list {
  width: 100%;
  margin: 0;
  padding: 0 64px 80px;
}

.setting-panel__section + .setting-panel__section {
  margin-top: 56px;
}

.setting-panel__section-header {
  margin-bottom: 16px;
  padding: 0;
  border-bottom: 1px solid var(--ui-border-subtle);
}

.setting-panel__section-title {
  margin: 0 0 12px;
  font-size: 14px;
  font-weight: var(--ui-font-weight-black);
  text-transform: uppercase;
  letter-spacing: 0.15em;
  color: var(--brand-primary);
  text-shadow: 0 0 12px color-mix(in srgb, var(--brand-primary), transparent 70%);
}

.setting-panel__section-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.setting-panel__notice {
  margin: 0 0 8px;
  padding: 10px 12px;
  border-left: 3px solid var(--color-warning);
  background: color-mix(in srgb, var(--color-warning), transparent 86%);
  color: color-mix(in srgb, var(--color-warning), var(--neutral-0) 20%);
  font-size: 13px;
  line-height: 1.5;
}

.setting-panel__summary {
  margin-bottom: 8px;
}

.setting-panel__action {
  min-height: 48px;
  padding: 0 16px;
  border: 1px solid var(--ui-border-subtle);
  border-radius: 8px;
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 13px;
  font-weight: var(--ui-font-weight-black);
  letter-spacing: 0.08em;
  text-transform: uppercase;
  transition: all var(--ui-motion-fast);
  text-align: center;
}

.setting-panel__action--danger {
  border-color: color-mix(in srgb, var(--color-danger), transparent 40%);
  color: var(--color-danger);
}

.setting-panel__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  border-color: color-mix(in srgb, var(--color-danger), transparent 50%);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-panel__action:disabled {
  opacity: 0.6;
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-settings-row-gap);
  width: 100%;
  min-height: 72px;
  padding: 12px 20px;
  border: 2px solid transparent;
  border-radius: 12px;
  background: var(--color-state-hover);
  color: var(--color-text-primary);
  text-align: left;
  transition: all var(--ui-motion-fast) var(--ease-standard);
}

.setting-row:hover {
  background: var(--color-state-hover);
}

.setting-row[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  z-index: 5;
}

.setting-row[data-focused='true'] .setting-row__label {
  color: var(--ui-focus-text);
}

.setting-row[data-focused='true'] .setting-row__desc {
  color: var(--color-text-secondary);
}

.setting-row[data-focused='true'] .setting-row__value {
  color: var(--brand-primary);
}

.setting-row__copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  justify-content: center;
  min-width: 0;
}

.setting-row__copy--singleline {
  min-height: 48px;
}

.setting-row__label {
  font-size: 18px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-bold);
  color: var(--color-text-primary);
}

.setting-row__badge {
  display: inline-block;
  margin-left: 8px;
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 11px;
  font-weight: var(--ui-font-weight-black);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  vertical-align: middle;
  background: color-mix(in srgb, var(--color-warning), transparent 78%);
  color: var(--color-warning);
}

.setting-row__desc {
  font-size: 14px;
  line-height: 1.5;
  color: var(--color-text-tertiary);
  opacity: 0.8;
}

.setting-row__value {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  font-size: 16px;
  font-weight: var(--ui-font-weight-black);
  letter-spacing: var(--letter-spacing-loose);
  color: var(--brand-primary);
  text-shadow: 0 0 12px color-mix(in srgb, var(--brand-primary), transparent 60%);
}

.setting-row--select .setting-row__value {
  color: var(--color-text-secondary);
}

.setting-row--select .setting-row__value::after {
  content: '›';
  display: inline-flex;
  align-items: center;
  margin-left: 12px;
  font-size: 22px;
  line-height: 1;
  color: var(--color-text-tertiary);
  transition: transform var(--ui-motion-fast) var(--ease-standard);
}
</style>
