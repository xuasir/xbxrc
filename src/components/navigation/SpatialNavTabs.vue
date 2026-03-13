<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'
import { computed } from 'vue'

interface SpatialNavTabItem {
  key: string
  label: string
  nodeId?: string
  disabled?: boolean
}

interface SpatialNavTabsProps {
  tabs: SpatialNavTabItem[]
  activeKey: string
  idPrefix?: string
  ariaLabel?: string
}

const props = withDefaults(defineProps<SpatialNavTabsProps>(), {
  idPrefix: 'tabs',
  ariaLabel: 'Tabs',
})

const emit = defineEmits<{
  (event: 'update:activeKey', key: string): void
  (event: 'change', key: string): void
}>()

interface ResolvedTabItem extends SpatialNavTabItem {
  nodeId: string
}

const resolvedTabs = computed<ResolvedTabItem[]>(() =>
  props.tabs.map(tab => ({
    ...tab,
    nodeId: tab.nodeId ?? `${props.idPrefix}.${tab.key}`,
  })),
)

function handleSelect(tabKey: string): void {
  emit('update:activeKey', tabKey)
  emit('change', tabKey)
}

function isActiveTab(tabKey: string): boolean {
  return props.activeKey === tabKey
}
</script>

<template>
  <nav class="sn-tabs" :aria-label="props.ariaLabel">
    <Focusable
      v-for="tab in resolvedTabs"
      :id="tab.nodeId"
      :key="tab.key"
      as="button"
      type="button"
      class="sn-tabs__item"
      :class="{ 'sn-tabs__item--active': isActiveTab(tab.key) }"
      :disabled="tab.disabled"
      :aria-label="tab.label"
      :on-confirm="() => handleSelect(tab.key)"
      @click="handleSelect(tab.key)"
    >
      <span class="sn-tabs__label">{{ tab.label }}</span>
    </Focusable>
  </nav>
</template>

<style scoped>
.sn-tabs {
  --tabs-underline-height: var(--ui-tabs-underline-height);
  --tabs-underline-inset: var(--ui-tabs-underline-inset);
  --tabs-underline-bottom-gap: var(--ui-tabs-underline-bottom-gap);
  --tabs-underline-radius: var(--ui-radius-pill);
  display: inline-flex;
  align-items: center;
  gap: var(--ui-tabs-gap);
}

.sn-tabs__item {
  position: relative;
  border: 1px solid transparent;
  border-radius: var(--ui-radius-sm);
  background: transparent;
  color: var(--ui-page-text-soft);
  font-family: var(--ui-font-family);
  font-size: var(--ui-tabs-font-size);
  font-weight: var(--ui-font-weight-semibold);
  line-height: var(--ui-line-height-tight);
  padding: var(--ui-tabs-padding);
  cursor: pointer;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    opacity var(--ui-motion-fast);
}

.sn-tabs__item::after {
  content: '';
  position: absolute;
  left: var(--tabs-underline-inset);
  right: var(--tabs-underline-inset);
  bottom: var(--tabs-underline-bottom-gap);
  height: var(--tabs-underline-height);
  border-radius: var(--tabs-underline-radius);
  background: transparent;
  transition: background-color var(--ui-motion-fast);
}

.sn-tabs__item--active {
  color: var(--ui-page-text);
  font-weight: var(--ui-font-weight-bold);
}

.sn-tabs__item--active::after {
  background: var(--brand-primary);
}

.sn-tabs__item[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.sn-tabs__item[data-focused='true']:not(.sn-tabs__item--active)::after {
  background: var(--ui-focus-text);
}

.sn-tabs__item:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.sn-tabs__label {
  display: inline-block;
}
</style>
