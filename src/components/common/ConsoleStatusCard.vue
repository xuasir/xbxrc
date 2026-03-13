<script setup lang="ts">
import type { Direction } from '@/navigation/core'
import { Focusable } from '@/navigation/core/vue'
import { computed } from 'vue'

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
  onClick?: () => void
  onConfirm?: () => void
}

const props = withDefaults(defineProps<ConsoleStatusCardProps>(), {
  imageAlt: '',
  ariaLabel: '',
  neighbors: undefined,
  index: undefined,
  disabled: false,
  onClick: undefined,
  onConfirm: undefined,
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
    :aria-label="resolvedAriaLabel"
    :on-confirm="props.onConfirm ?? props.onClick"
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
.console-status-card {
  position: relative;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  flex: 0 0 clamp(var(--ui-console-card-min-width), 24vw, var(--ui-console-card-width));
  width: clamp(var(--ui-console-card-min-width), 24vw, var(--ui-console-card-width));
  min-height: clamp(var(--ui-console-card-min-height-min), 30vw, var(--ui-console-card-min-height));
  padding: var(--ui-console-card-padding);
  border-radius: var(--ui-console-card-radius);
  border: 1px solid var(--tile-border);
  background: #2b2b2b;
  color: var(--color-text-primary);
  text-align: left;
  overflow: hidden;
  cursor: pointer;
  box-shadow: 0 10px 20px rgba(0, 0, 0, 0.2);
  transition:
    border-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.console-status-card[data-focused='true'] {
  background: var(--color-focus-bg);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
  z-index: 10;
}

.console-status-card__media,
.console-status-card__body {
  position: relative;
  z-index: 1;
}

.console-status-card__media {
  display: flex;
  justify-content: center;
  align-items: flex-start;
  min-height: var(--ui-console-card-media-height);
  padding-top: 2px;
}

.console-status-card__image {
  width: min(
    100%,
    clamp(var(--ui-console-card-image-min-width), 15vw, var(--ui-console-card-image-width))
  );
  height: auto;
  object-fit: contain;
  filter: drop-shadow(0 16px 18px rgba(0, 0, 0, 0.34)) drop-shadow(0 6px 8px rgba(0, 0, 0, 0.18));
}

.console-status-card__body {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: auto;
}

.console-status-card__title {
  font-size: var(--ui-console-card-title-size);
  line-height: 1;
  font-weight: var(--ui-font-weight-bold);
  letter-spacing: -0.03em;
  color: var(--color-text-primary);
}

.console-status-card__status {
  display: -webkit-box;
  overflow: hidden;
  font-size: var(--ui-console-card-status-size);
  line-height: 1.2;
  font-weight: var(--ui-font-weight-medium);
  color: var(--color-text-secondary);
  text-overflow: ellipsis;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

.console-status-card__description {
  display: -webkit-box;
  overflow: hidden;
  margin-top: 10px;
  font-size: var(--ui-console-card-description-size);
  line-height: 1.18;
  font-weight: var(--ui-font-weight-medium);
  color: color-mix(in srgb, var(--color-text-primary) 86%, transparent);
  text-overflow: ellipsis;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

:global(html[data-ui-density='compact']) .console-status-card__description,
:global(html[data-ui-density='narrow']) .console-status-card__description {
  margin-top: 8px;
}
</style>
