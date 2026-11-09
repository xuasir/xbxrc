<script setup lang="ts">
import type { Direction } from '@/navigation/core'
import { computed } from 'vue'
import { Focusable } from '@/navigation/core/vue'

interface SpatialNavNodeIndex {
  row?: number
  col?: number
  order?: number
}

type SpatialNavNeighbors = Partial<Record<Direction, string>>

interface ConsoleStatusCardProps {
  id: string
  scopeId: string
  title: string
  status: string
  description: string
  imageSrc: string
  imageAlt?: string
  ariaLabel?: string
  neighbors?: SpatialNavNeighbors
  index?: SpatialNavNodeIndex
  disabled?: boolean
  selected?: boolean
  onClick?: () => void
}

const props = withDefaults(defineProps<ConsoleStatusCardProps>(), {
  imageAlt: '',
  ariaLabel: '',
  neighbors: undefined,
  index: undefined,
  disabled: false,
  selected: false,
  onClick: undefined,
})

// 统一生成可读的无障碍文案，避免页面每次重复拼接。
const resolvedAriaLabel = computed(() => {
  return props.ariaLabel || `${props.title}. ${props.status}. ${props.description}.`
})
</script>

<template>
  <Focusable
    :id="props.id"
    as="button"
    type="button"
    class="console-status-card"
    :scope-id="props.scopeId"
    :disabled="props.disabled"
    :aria-pressed="props.selected"
    :aria-label="resolvedAriaLabel"
    @click="props.onClick"
  >
    <span class="console-status-card__media" aria-hidden="true">
      <img class="console-status-card__image" :src="props.imageSrc" :alt="props.imageAlt">
    </span>

    <span class="console-status-card__body">
      <span class="console-status-card__title">{{ props.title }}</span>
      <span class="console-status-card__status">{{ props.status }}</span>
      <span class="console-status-card__description">{{ props.description }}</span>
    </span>
  </Focusable>
</template>

<style scoped>
.console-status-card { --focus-ring-radius: var(--ui-radius-lg); position: relative; display: flex; align-items: center; gap: var(--space-12); min-width: 0; width: 100%; min-height: 80px; padding: var(--space-8) var(--space-12); border: 1px solid var(--ui-border-subtle); border-radius: var(--focus-ring-radius); background: var(--ui-material-glass); color: var(--color-text-primary); text-align: left; cursor: pointer; box-shadow: inset 0 1px 0 var(--ui-material-edge), var(--ui-shadow-low); transition: background var(--ui-motion-fast), border-color var(--ui-motion-fast), box-shadow var(--ui-motion-fast); }
.console-status-card[aria-pressed='true'] { background: var(--ui-choice-selected-bg); border-color: var(--ui-choice-selected-border); box-shadow: none; }
.console-status-card:hover { background: var(--ui-material-hover); border-color: transparent; }
.console-status-card[aria-pressed='true']:hover { background: var(--ui-choice-selected-bg); border-color: var(--ui-choice-selected-border); }
.console-status-card[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); z-index: 1; }
.console-status-card__media { flex: 0 0 48px; height: 60px; }
.console-status-card__image { display: block; width: 100%; height: 100%; object-fit: contain; }
.console-status-card__body { display: flex; flex-direction: column; gap: var(--space-4); min-width: 0; }
.console-status-card__title { font-size: 16px; line-height: 20px; font-weight: 500; overflow-wrap: anywhere; }
.console-status-card__status, .console-status-card__description { font-size: 12px; line-height: 18px; color: var(--color-text-secondary); }
.console-status-card__description { display: none; }
</style>
