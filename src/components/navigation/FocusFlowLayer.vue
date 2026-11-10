<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useUiMotion } from '@/app/ui-motion'
import { navigationEngine } from '@/navigation/core/engine'
import { createFocusFlow } from '@/navigation/focus-flow'

const ring = ref<HTMLElement | null>(null)
const motion = useUiMotion()
const route = useRoute()
let flow: ReturnType<typeof createFocusFlow> | undefined
onMounted(() => {
  if (ring.value)
    flow = createFocusFlow(navigationEngine, ring.value, () => motion?.enabled.value === true)
})
watch(() => route.fullPath, () => flow?.reset(), { flush: 'sync' })
watch(() => motion?.enabled.value, () => flow?.refresh(), { flush: 'sync' })
onBeforeUnmount(() => flow?.dispose())
</script>

<template>
  <Teleport to="body">
    <div class="focus-flow-layer" aria-hidden="true">
      <div ref="ring" class="focus-flow-ring" hidden />
    </div>
  </Teleport>
</template>

<style scoped>
.focus-flow-layer { position: fixed; inset: 0; z-index: calc(var(--z-toast) - 1); pointer-events: none; }
.focus-flow-ring { position: absolute; padding: var(--focus-ring-width); background: conic-gradient(from var(--ui-focus-angle), var(--ui-focus-ring-start), var(--ui-focus-ring-end), var(--ui-focus-ring-start)); -webkit-mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0); -webkit-mask-composite: xor; mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0); mask-composite: exclude; pointer-events: none; }
</style>
