<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'

interface SpatialNavIconButtonProps {
  id: string
  label: string
  iconSrc?: string
  iconAlt?: string
  round?: boolean
  active?: boolean
  onClick?: () => void
}

const props = withDefaults(defineProps<SpatialNavIconButtonProps>(), {
  iconSrc: '',
  iconAlt: '',
  round: false,
  active: false,
  onClick: undefined,
})
</script>

<template>
  <Focusable
    :id="props.id"
    as="button"
    type="button"
    class="sn-icon-button"
    :class="{ 'sn-icon-button--round': props.round, 'sn-icon-button--active': props.active }"
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
        >
        <span v-else class="sn-icon-button__icon-empty" aria-hidden="true" />
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
  background: var(--color-focus-bg);
  color: var(--ui-focus-text);
  /* box-shadow: var(--shadow-xbox-focus); */
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
