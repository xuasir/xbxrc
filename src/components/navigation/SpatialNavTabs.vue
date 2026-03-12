<script setup lang="ts">
import type { NodeDef } from '@spatial-navigation/runtime'
import { Focusable } from '@spatial-navigation/vue'
import { computed } from 'vue'
import { SPATIAL_NAV_TAB_LEVELS } from '../../navigation/spatial-nav.constants'

interface SpatialNavTabItem {
  key: string
  label: string
  nodeId?: string
  disabled?: boolean
}

interface SpatialNavTabsProps {
  scopeId: string
  tabs: SpatialNavTabItem[]
  activeKey: string
  idPrefix?: string
  tabLevel?: NodeDef['tabLevel']
  upNeighborId?: string
  downNeighborId?: string
  ariaLabel?: string
}

const props = withDefaults(defineProps<SpatialNavTabsProps>(), {
  idPrefix: 'tabs',
  tabLevel: SPATIAL_NAV_TAB_LEVELS.secondary,
  upNeighborId: undefined,
  downNeighborId: undefined,
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

function findPrevEnabledNodeId(startIndex: number): string | undefined {
  for (let index = startIndex; index >= 0; index -= 1) {
    const tab = resolvedTabs.value[index]
    if (!tab.disabled) {
      return tab.nodeId
    }
  }
  return undefined
}

function findNextEnabledNodeId(startIndex: number): string | undefined {
  for (let index = startIndex; index < resolvedTabs.value.length; index += 1) {
    const tab = resolvedTabs.value[index]
    if (!tab.disabled) {
      return tab.nodeId
    }
  }
  return undefined
}

// 通过显式邻接关系保证方向键行为可预测，同时保留 TAB_NAV（LT/RT）切换
function buildNeighbors(index: number): NodeDef['neighbors'] {
  const neighbors: NodeDef['neighbors'] = {}
  const leftId = findPrevEnabledNodeId(index - 1)
  const rightId = findNextEnabledNodeId(index + 1)

  if (leftId !== undefined) {
    neighbors.left = leftId
  }
  if (rightId !== undefined) {
    neighbors.right = rightId
  }
  if (props.upNeighborId !== undefined) {
    neighbors.up = props.upNeighborId
  }
  if (props.downNeighborId !== undefined) {
    neighbors.down = props.downNeighborId
  }

  return neighbors
}

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
      v-for="(tab, index) in resolvedTabs"
      :id="tab.nodeId"
      :key="tab.key"
      as="button"
      type="button"
      class="sn-tabs__item"
      :class="{ 'sn-tabs__item--active': isActiveTab(tab.key) }"
      :scope-id="props.scopeId"
      :disabled="tab.disabled"
      :neighbors="buildNeighbors(index)"
      :tab-level="props.tabLevel"
      :index="{ order: index }"
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
  background: #107c10;
}

.sn-tabs__item[data-focused='true'] {
  background: transparent;
  box-shadow: var(--ui-focus-ring-shadow);
  color: var(--ui-page-text);
}

.sn-tabs__item[data-focused='true']:not(.sn-tabs__item--active)::after {
  background: #ffffff;
}

.sn-tabs__item:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.sn-tabs__label {
  display: inline-block;
}
</style>
