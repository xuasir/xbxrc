<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'

defineProps<{ id: string, busy?: boolean }>()
defineEmits<{ refresh: [] }>()
</script>

<template>
  <Focusable :id="id" as="button" type="button" class="library-refresh" :class="{ 'library-refresh--busy': busy }" aria-label="刷新" title="刷新" :aria-busy="busy" :busy="busy" @click="$emit('refresh')">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5" /><path d="M6.2 6.2A8 8 0 0 1 20 12M4 12a8 8 0 0 0 13.8 5.8" /></svg>
  </Focusable>
</template>

<style scoped>
.library-refresh { display:grid; place-items:center; flex:0 0 36px; width:36px; height:36px; padding:8px; border:1px solid transparent; border-radius:var(--ui-radius-md); background:transparent; color:var(--color-text-primary); cursor:pointer; }
.library-refresh svg { width:20px; height:20px; }
.library-refresh:disabled { opacity:.5; cursor:default; }
.library-refresh:hover:enabled { background:var(--ui-material-hover); }
.library-refresh[data-focused='true'] { color:var(--ui-focus-text); background:var(--ui-material-hover); box-shadow:var(--shadow-xbox-focus); }
.library-refresh--busy svg { animation:library-refresh-spin 900ms linear infinite; }
@keyframes library-refresh-spin { to { transform:rotate(360deg); } }
@media(prefers-reduced-motion:reduce) { .library-refresh--busy svg { animation:none; } }
</style>
