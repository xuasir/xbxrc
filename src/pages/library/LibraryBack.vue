<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'

defineProps<{ id: string, label?: string }>()
defineEmits<{ back: [] }>()
</script>

<template>
  <Focusable :id="id" as="button" type="button" class="library-back" data-library-entry @click="$emit('back')">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m14 6-6 6 6 6" /></svg>
    <span>{{ label || '返回' }}</span>
  </Focusable>
</template>

<style scoped>
.library-back { display:inline-flex; align-items:center; gap:var(--space-4); min-height:36px; padding:var(--space-4) var(--space-8) var(--space-4) 0; border:1px solid transparent; border-radius:var(--ui-radius-md); background:transparent; color:var(--color-text-primary); font:inherit; font-size:var(--ui-text-body-sm); line-height:var(--ui-line-height-relaxed); cursor:pointer; transition:background var(--ui-motion-fast), color var(--ui-motion-fast), box-shadow var(--ui-motion-fast); }
.library-back svg { width:20px; height:20px; flex-shrink:0; }
.library-back:hover { color:var(--color-text-primary); background:var(--ui-material-hover); }
.library-back[data-focused='true'] { color:var(--ui-focus-text); background:var(--ui-material-hover); box-shadow:var(--shadow-xbox-focus); }
</style>
