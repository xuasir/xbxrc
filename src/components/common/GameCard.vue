<script setup lang="ts">
import type { Direction } from '@spatial-navigation/runtime'
import { Focusable } from '@spatial-navigation/vue'

type SpatialNavNeighbors = Partial<Record<Direction, string>>

interface SpatialNavNodeIndex {
  row?: number
  col?: number
  order?: number
}

interface GameCardProps {
  id: string
  scopeId: string
  title: string
  imageUrl?: string
  ariaLabel?: string
  disabled?: boolean
  neighbors?: SpatialNavNeighbors
  index?: SpatialNavNodeIndex
}

const props = withDefaults(defineProps<GameCardProps>(), {
  imageUrl: '',
  ariaLabel: '',
  disabled: false,
  neighbors: undefined,
  index: undefined,
})

const emit = defineEmits<{
  (event: 'select'): void
}>()

// 统一收敛点击和确认事件，页面层只处理一次选择回调。
function handleSelect(): void {
  emit('select')
}
</script>

<template>
  <Focusable
    :id="props.id"
    as="button"
    type="button"
    class="game-card"
    :scope-id="props.scopeId"
    :disabled="props.disabled"
    :neighbors="props.neighbors"
    :index="props.index"
    :aria-label="props.ariaLabel || props.title"
    :on-confirm="handleSelect"
    @click="handleSelect"
  >
    <span class="game-card__image-shell">
      <span
        class="game-card__image"
        :style="{ backgroundImage: props.imageUrl ? `url(${props.imageUrl})` : undefined }"
        aria-hidden="true"
      />
      <span class="game-card__title-overlay">
        <span class="game-card__title-text">{{ props.title }}</span>
      </span>
    </span>
  </Focusable>
</template>

<style scoped>
.game-card {
  flex: 0 0 var(--ui-game-card-size);
  width: var(--ui-game-card-size);
  height: var(--ui-game-card-size);
  padding: 0;
  border: 1px solid color-mix(in srgb, var(--color-border-subtle) 86%, rgba(255, 255, 255, 0.06));
  border-radius: var(--ui-game-card-radius);
  background: color-mix(in srgb, var(--color-surface-2) 94%, rgba(0, 0, 0, 0.12));
  color: var(--color-text-primary);
  text-align: left;
  cursor: pointer;
  overflow: hidden;
  transition:
    transform var(--ui-motion-fast),
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    filter var(--ui-motion-fast);
}

.game-card:hover {
  filter: brightness(1.03);
}

.game-card[data-focused='true'] {
  background: var(--color-surface-2);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.04);
  z-index: 10;
}

.game-card__image-shell,
.game-card__image {
  display: block;
  width: 100%;
  height: 100%;
}

.game-card__image-shell {
  position: relative;
  overflow: hidden;
}

.game-card__image {
  position: relative;
  z-index: 1;
  background-color: color-mix(in srgb, var(--color-surface-3) 94%, rgba(0, 0, 0, 0.18));
  background-position: center;
  background-repeat: no-repeat;
  background-size: cover;
  transform: scale(1.01);
  transition:
    transform 220ms ease,
    filter 220ms ease;
}

.game-card__title-overlay {
  position: absolute;
  right: 0;
  bottom: 0;
  left: 0;
  z-index: 2;
  display: flex;
  align-items: flex-end;
  min-height: var(--ui-game-card-title-min-height);
  padding: var(--ui-game-card-title-padding);
  background: linear-gradient(180deg, transparent, rgba(0, 0, 0, 0.72) 40%, rgba(0, 0, 0, 0.94));
  opacity: 0.92;
  transform: translateY(6px);
  transition:
    opacity var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.game-card__title-text {
  display: -webkit-box;
  overflow: hidden;
  font-size: var(--ui-game-card-title-font-size);
  line-height: 1.2;
  font-weight: var(--ui-font-weight-bold);
  color: var(--color-text-on-media);
  text-overflow: ellipsis;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

.game-card:hover .game-card__title-overlay,
.game-card[data-focused='true'] .game-card__title-overlay {
  opacity: 1;
  transform: translateY(0);
}

.game-card:hover .game-card__image,
.game-card[data-focused='true'] .game-card__image {
  filter: saturate(1.08);
}
</style>
