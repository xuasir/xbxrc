<script setup lang="ts">
import type { Direction, TabLevel } from '@spatial-navigation/runtime'
import { Focusable } from '@spatial-navigation/vue'

interface SpatialNavNodeIndex {
  row?: number
  col?: number
  order?: number
}

type SpatialNavNeighbors = Partial<Record<Direction, string>>

interface SpatialNavIconButtonProps {
  id: string
  scopeId: string
  label: string
  neighbors?: SpatialNavNeighbors
  tabLevel?: TabLevel
  index?: SpatialNavNodeIndex
  iconSrc?: string
  iconAlt?: string
  round?: boolean
  active?: boolean
  onClick?: () => void
  onConfirm?: () => void
}

const props = withDefaults(defineProps<SpatialNavIconButtonProps>(), {
  neighbors: undefined,
  tabLevel: undefined,
  index: undefined,
  iconSrc: '',
  iconAlt: '',
  round: false,
  active: false,
  onClick: undefined,
  onConfirm: undefined
})
</script>

<template>
  <Focusable
    :id="props.id"
    as="button"
    type="button"
    class="sn-icon-button"
    :class="{ 'sn-icon-button--round': props.round, 'sn-icon-button--active': props.active }"
    :scope-id="props.scopeId"
    :neighbors="props.neighbors"
    :tab-level="props.tabLevel"
    :index="props.index"
    :on-confirm="props.onConfirm"
    :aria-label="props.label"
    @click="props.onClick"
  >
    <slot>
      <span class="sn-icon-button__icon-shell">
        <img
          v-if="props.iconSrc"
          class="sn-icon-button__icon"
          :src="props.iconSrc"
          :alt="props.iconAlt || props.label"
        />
        <span v-else class="sn-icon-button__icon-empty" aria-hidden="true"></span>
      </span>
    </slot>
  </Focusable>
</template>

<style scoped>
.sn-icon-button {
  width: var(--ui-size-control-lg);
  height: var(--ui-size-control-lg);
  padding: 3px;
  border: 1px solid transparent;
  border-radius: var(--btn-radius);
  background: transparent;
  color: var(--color-text-primary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast);
}

.sn-icon-button--round {
  border-radius: 999px;
}

.sn-icon-button[data-focused='true'] {
  border-color: var(--color-focus-ring);
  background: color-mix(in srgb, var(--color-state-hover) 72%, transparent);
  box-shadow: 0 0 0 var(--focus-ring-width) var(--color-focus-ring-outer) inset;
}

.sn-icon-button--active {
  background: var(--color-state-selected);
}

.sn-icon-button__icon-shell {
  width: var(--ui-size-icon-sm);
  height: var(--ui-size-icon-sm);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.sn-icon-button__icon {
  width: var(--ui-size-icon-sm);
  height: var(--ui-size-icon-sm);
  object-fit: contain;
  display: block;
  filter: var(--ui-nav-icon-filter);
}

.sn-icon-button__icon-empty {
  width: var(--ui-size-icon-sm);
  height: var(--ui-size-icon-sm);
  border-radius: var(--pill-radius);
  border: 1px dashed var(--color-border-subtle);
  background: var(--color-surface-2);
}
</style>
