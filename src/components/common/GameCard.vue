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
  actions?: boolean
  favorite?: boolean
  playBusy?: boolean
  favoriteBusy?: boolean
  playDisabled?: boolean
  playLabel?: string
  favoriteLabel?: string
  meta?: string
  imageSources?: string[]
  badge?: string
  imageUrl?: string
  ariaLabel?: string
  disabled?: boolean
  neighbors?: SpatialNavNeighbors
  index?: SpatialNavNodeIndex
}

const props = withDefaults(defineProps<GameCardProps>(), {
  actions: false,
  favorite: false,
  playBusy: false,
  favoriteBusy: false,
  playDisabled: false,
  playLabel: 'Play',
  favoriteLabel: 'Favorite',
  meta: '',
  imageSources: () => [],
  badge: '',
  imageUrl: '',
  ariaLabel: '',
  disabled: false,
  neighbors: undefined,
  index: undefined,
})

const emit = defineEmits<{
  (event: 'select'): void
  (event: 'play'): void
  (event: 'favorite'): void
}>()

// 统一收敛点击和确认事件，页面层只处理一次选择回调。
function handleSelect(): void {
  emit('select')
}
</script>

<template>
  <div class="game-card">
    <Focusable :id="props.id" as="button" type="button" class="game-card__main" :scope-id="props.scopeId" :disabled="props.disabled" :aria-label="props.ariaLabel || props.title" @click="handleSelect">
      <span class="game-card__image-shell">
        <RemoteImage class="game-card__image" :src="props.imageUrl" :sources="props.imageSources" />
        <span v-if="props.badge" class="game-card__badge">{{ props.badge }}</span>
        <span class="game-card__title-overlay">
          <span v-if="props.meta" class="game-card__meta">{{ props.meta }}</span>
          <span class="game-card__title-text">{{ props.title }}</span>
        </span>
      </span>
    </Focusable>
    <span v-if="props.actions" class="game-card__actions">
      <Focusable :id="`${props.id}.play`" as="button" type="button" class="game-card__icon" :scope-id="props.scopeId" :disabled="props.playDisabled" :busy="props.playBusy" :aria-label="props.playLabel" :title="props.playLabel" @click="emit('play')">
        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M8 5.5a1 1 0 0 1 1.5-.86l10 6.5a1 1 0 0 1 0 1.72l-10 6.5A1 1 0 0 1 8 18.5z" /></svg>
      </Focusable>
      <Focusable :id="`${props.id}.favorite`" as="button" type="button" class="game-card__icon" :class="{ 'game-card__icon--active': props.favorite }" :scope-id="props.scopeId" :busy="props.favoriteBusy" :aria-pressed="props.favorite" :aria-label="props.favoriteLabel" :title="props.favoriteLabel" @click="emit('favorite')">
        <svg viewBox="0 0 24 24" :fill="props.favorite ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20.5 5.5a5 5 0 0 0-7 0L12 7l-1.5-1.5a5 5 0 0 0-7 7L12 21l8.5-8.5a5 5 0 0 0 0-7Z" /></svg>
      </Focusable>
    </span>
  </div>
</template>

<style scoped>
.game-card__main { position: absolute; inset: 0; display: block; width: 100%; height: 100%; padding: 0; border: 0; border-radius: inherit; background: transparent; color: inherit; cursor: pointer; text-align: left; }
.game-card__actions { position: absolute; top: 8px; right: 8px; z-index: 4; display: flex; gap: 5px; }
.game-card__icon { display: grid; place-items: center; width: 32px; height: 32px; padding: 7px; border: 1px solid rgba(255,255,255,.18); border-radius: 50%; color: #fff; background: rgba(12,18,22,.78); backdrop-filter: blur(12px); cursor: pointer; transition: background-color 150ms, transform 150ms; }
.game-card__icon svg { width: 100%; height: 100%; }
.game-card__icon:hover { background: rgba(35,65,48,.96); transform: translateY(-1px); }
.game-card__icon--active { background: rgba(34,108,53,.94); border-color: rgba(138,228,145,.38); }
.game-card__icon[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.game-card__icon:disabled { opacity: .4; cursor: default; }
.game-card__icon[aria-busy='true'] { opacity: .6; }
.game-card__meta { color: #c1e4c7; font-size: 11px; line-height: 1.2; }

.game-card__badge { position: absolute; top: 9px; left: 8px; max-width: calc(100% - 86px); width: fit-content; z-index: 3; border-radius: var(--ui-radius-md); padding: 4px 7px; color: var(--color-text-on-media); background: rgba(0,0,0,.75); font-size: 11px; line-height: 1.4; }
.game-card {
  position: relative;
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

.game-card:has(.game-card__main[data-focused='true']) {
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
  align-items: flex-start;
  flex-direction: column;
  gap: 4px;
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
.game-card:has(.game-card__main[data-focused='true']) .game-card__title-overlay {
  opacity: 1;
  transform: translateY(0);
}

.game-card:hover .game-card__image,
.game-card:has(.game-card__main[data-focused='true']) .game-card__image {
  filter: saturate(1.08);
}
</style>
