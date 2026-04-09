<script setup lang="ts">
import type { SettingTabKey } from '../../navigation/spatial-nav.constants'
import type { SettingIndexedSection, SettingRow } from './setting-types'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingInlineSingleSelect from '../../components/settings/SettingInlineSingleSelect.vue'
import SettingToggleRow from '../../components/settings/SettingToggleRow.vue'
import { resolveUiLocale } from '../../i18n'

const props = defineProps<{
  activeTabKey: SettingTabKey
  sections: SettingIndexedSection[]
  scopeId: string
  pendingActionKey: string | null
  activeInlineSingleSelectRowNodeId: string | null
  streamingExpertResetNodeId: string
  isStreamingExpertResetPending: boolean
}>()

const emit = defineEmits<{
  rowConfirm: [row: SettingRow]
  closeInlineSingleSelect: []
  inlineSingleSelect: [value: string | number]
  resetStreamingExpert: []
}>()

const { t } = useI18n()
</script>

<template>
  <div class="setting-panel__list">
    <section
      v-for="section in props.sections"
      :key="section.key"
      class="setting-panel__section"
      :aria-label="section.label"
    >
      <header
        class="setting-panel__section-header"
        :class="{
          'setting-panel__section-header--expert':
            props.activeTabKey === 'streaming' && section.key === 'expert',
        }"
      >
        <h2 class="setting-panel__section-title">
          {{ section.label }}
        </h2>
        <Focusable
          v-if="props.activeTabKey === 'streaming' && section.key === 'expert'"
          :id="props.streamingExpertResetNodeId"
          as="button"
          type="button"
          class="setting-panel__expert-reset"
          :scope-id="props.scopeId"
          :aria-label="t('setting.streaming.expert.reset')"
          :disabled="props.pendingActionKey !== null"
          @click="emit('resetStreamingExpert')"
        >
          {{
            props.isStreamingExpertResetPending
              ? t('setting.streaming.expert.resetting')
              : t('setting.streaming.expert.reset')
          }}
        </Focusable>
      </header>
      <p
        v-if="props.activeTabKey === 'streaming' && section.key === 'expert'"
        class="setting-panel__expert-risk"
      >
        {{ t('setting.streaming.expert.riskHint') }}
      </p>

      <div class="setting-panel__section-body">
        <template v-for="row in section.rows" :key="row.nodeId">
          <SettingToggleRow
            v-if="row.control === 'toggle'"
            :id="row.nodeId"
            :scope-id="props.scopeId"
            :label="row.label"
            :enabled="row.value === true"
            :order="row.index"
            @confirm="emit('rowConfirm', row)"
          />

          <Focusable
            v-else
            :id="row.nodeId"
            as="button"
            type="button"
            class="setting-row"
            :class="{
              'setting-row--select': row.control === 'singleSelect',
              'setting-row--inline-expanded':
                row.control === 'singleSelect'
                && (row.options?.length ?? 0) <= 3
                && props.activeInlineSingleSelectRowNodeId === row.nodeId,
            }"
            :scope-id="props.scopeId"
            :aria-label="row.label"
            :on-back="
              row.control === 'singleSelect'
                && (row.options?.length ?? 0) <= 3
                && props.activeInlineSingleSelectRowNodeId === row.nodeId
                ? () => {
                  emit('closeInlineSingleSelect')
                }
                : undefined
            "
            @click="emit('rowConfirm', row)"
          >
            <span class="setting-row__copy">
              <span class="setting-row__label">{{ row.label }}</span>
              <span v-if="row.description" class="setting-row__desc">{{ row.description }}</span>
            </span>
            <span class="setting-row__value">{{ row.valueText }}</span>
          </Focusable>

          <SettingInlineSingleSelect
            v-if="
              row.control === 'singleSelect'
                && (row.options?.length ?? 0) <= 3
                && props.activeInlineSingleSelectRowNodeId === row.nodeId
            "
            :open="true"
            :scope-id="props.scopeId"
            :row-node-id="row.nodeId"
            :options="row.options ?? []"
            :current-value="
              typeof row.value === 'string' || typeof row.value === 'number'
                ? row.key === 'locale'
                  ? resolveUiLocale(row.value)
                  : row.value
                : null
            "
            :disabled="props.pendingActionKey !== null"
            @close="emit('closeInlineSingleSelect')"
            @select="(value) => emit('inlineSingleSelect', value)"
          />
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

.setting-panel__section-header--expert {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
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

.setting-panel__expert-reset {
  margin-bottom: 10px;
  min-height: 34px;
  padding: 0 12px;
  border: 1px solid var(--ui-border-subtle);
  border-radius: 8px;
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 12px;
  font-weight: var(--ui-font-weight-black);
  letter-spacing: 0.08em;
  text-transform: uppercase;
  transition: all var(--ui-motion-fast);
}

.setting-panel__expert-reset[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  border-color: color-mix(in srgb, var(--color-danger), transparent 50%);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-panel__expert-reset:disabled {
  opacity: 0.6;
}

.setting-panel__expert-risk {
  margin: -4px 0 14px;
  padding: 10px 12px;
  border-left: 3px solid var(--color-warning);
  background: color-mix(in srgb, var(--color-warning), transparent 86%);
  color: color-mix(in srgb, var(--color-warning), var(--neutral-0) 20%);
  font-size: 13px;
  line-height: 1.5;
}

.setting-panel__section-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
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
  min-width: 0;
}

.setting-row__label {
  font-size: 18px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-bold);
  color: var(--color-text-primary);
}

.setting-row__desc {
  font-size: 14px;
  line-height: 1.5;
  color: var(--color-text-tertiary);
  opacity: 0.8;
}

.setting-row__value {
  flex: 0 0 auto;
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
  display: inline-block;
  margin-left: 12px;
  font-size: 22px;
  line-height: 1;
  color: var(--color-text-tertiary);
  vertical-align: middle;
  transition: transform var(--ui-motion-fast) var(--ease-standard);
}

.setting-row--inline-expanded {
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
}

.setting-row--inline-expanded.setting-row--select .setting-row__value::after {
  transform: rotate(90deg);
  color: var(--brand-primary);
}
</style>
