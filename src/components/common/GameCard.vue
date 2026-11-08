<script setup lang="ts">
import type { Direction } from '@/navigation/core'
import { Focusable } from '@/navigation/core/vue'
import RemoteImage from './RemoteImage.vue'

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
    :aria-label="props.ariaLabel || props.title"
    @click="handleSelect"
  >
    <span class="game-card__image-shell">
      <RemoteImage
        class="game-card__image"
        :src="props.imageUrl"
      />
      <span class="game-card__title-overlay">
        <span class="game-card__title-text">{{ props.title }}</span>
      </span>
    </span>
  </Focusable>
</template>

<style scoped>
.game-card {
  --focus-ring-radius: var(--ui-game-card-radius);
  flex: 0 0 var(--ui-game-card-size);
  width: var(--ui-game-card-size);
  height: var(--ui-game-card-size);
  padding: 0;
  border: 1px solid transparent;
  border-radius: var(--focus-ring-radius);
  background: transparent;
  color: var(--color-text-primary);
  text-align: left;
  cursor: pointer;
  overflow: visible;
  transition:
    transform var(--ui-motion-fast),
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    filter var(--ui-motion-fast);
}

.game-card:hover {
  border-color: var(--ui-border-hover);
  box-shadow: 0 0 16px color-mix(in srgb, var(--ui-accent-glow) 12%, transparent);
}

.game-card[data-focused='true'] {
  background: transparent;
  box-shadow: var(--shadow-xbox-focus);
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
  border-radius: inherit;
}

.game-card__image {
  position: relative;
  z-index: 1;
  background: transparent;
  object-fit: contain;
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
  background: linear-gradient(
    180deg,
    transparent,
    color-mix(in srgb, var(--neutral-0) 72%, transparent) 40%,
    color-mix(in srgb, var(--neutral-0) 94%, transparent)
  );
  opacity: 1;
  transition:
    opacity var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.game-card__title-text {
  display: -webkit-box;
  overflow: hidden;
  font-size: var(--ui-game-card-title-font-size);
  line-height: 1.2;
  font-weight: 400;
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
