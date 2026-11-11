<script setup lang="ts">
import { computed, onActivated, ref, watch } from 'vue'

const props = withDefaults(defineProps<{ src?: string | null, sources?: string[], alt?: string, fallback?: string }>(), { src: '', sources: () => [], alt: '', fallback: '' })
const candidates = computed(() => [...new Set([props.src ?? '', ...props.sources].map(value => value.trim()).filter(Boolean))])
const candidate = ref(0)
const source = computed(() => candidates.value[candidate.value] ?? '')
const failed = computed(() => !source.value)
const loaded = ref(false)
watch(() => candidates.value.join('\n'), () => {
  candidate.value = 0
  loaded.value = false
})
function imageError(event: Event) {
  if ((event.currentTarget as HTMLElement).dataset.source === source.value) {
    candidate.value++
    loaded.value = false
  }
}
function imageLoaded(event: Event) {
  if ((event.currentTarget as HTMLElement).dataset.source === source.value)
    loaded.value = true
}
onActivated(() => {
  if (failed.value) {
    candidate.value = 0
    loaded.value = false
  }
})
</script>

<template>
  <span class="remote-image" :data-image-state="!source || failed ? 'unavailable' : loaded ? 'loaded' : 'loading'" :role="alt ? 'img' : undefined" :aria-label="alt || undefined" :aria-hidden="alt ? undefined : true">
    <span v-if="!loaded || failed" class="remote-image__fallback" aria-hidden="true">
      <span v-if="fallback">{{ fallback }}</span>
      <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M7.5 7h9c2 0 3 1.5 3.5 3.5l1 5c.5 2.5-2 4-3.5 2.5L15 15H9l-2.5 3c-1.5 1.5-4 0-3.5-2.5l1-5C4.5 8.5 5.5 7 7.5 7Z" /><path d="M6 11h4M8 9v4M16 10h.01M18 12h.01" /></svg>
    </span>
    <img v-if="source && !failed" :key="source" :data-source="source" class="remote-image__image" :src="source" alt="" loading="lazy" decoding="async" @load="imageLoaded" @error="imageError">
  </span>
</template>

<style scoped>
.remote-image { position: relative; display: block; overflow: hidden; object-fit: contain; }
.remote-image__image { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: inherit; }
.remote-image__fallback { position: absolute; inset: 0; display: grid; place-items: center; color: var(--color-text-tertiary); background: linear-gradient(145deg, var(--ui-material-selected), var(--ui-panel-bottom)); }
.remote-image__fallback svg { width: 42%; max-width: 56px; height: auto; }
</style>
