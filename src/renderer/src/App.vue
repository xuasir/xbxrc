<script setup lang="ts">
import { ConsoleUIProvider } from '@spatial-navigation/vue'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { setupUiDensity } from './app/ui-density'
import AppShellLayout from './components/layout/AppShellLayout.vue'
import SpatialNavGlobalHotkeys from './components/navigation/SpatialNavGlobalHotkeys.vue'
import { events } from './services/events'
import { rpc } from './services/rpc'

const route = useRoute()
let disposeAuthSessionReady: (() => void) | undefined
let disposeUiDensity: (() => void) | undefined
let xcloudWarmupPromise: Promise<unknown> | undefined
const streamUiInputEnabled = ref(false)

function handleStreamUiInputMode(event: Event): void {
  const detail = (event as CustomEvent<{ enabled?: boolean }>).detail
  streamUiInputEnabled.value = detail?.enabled === true
}

const isStreamRoute = computed(
  () => route.name === 'xhome-stream' || route.name === 'xcloud-stream'
)

const spatialInputSources = computed(() => {
  if (!isStreamRoute.value) {
    return { keyboard: true, gamepad: true }
  }
  return {
    keyboard: streamUiInputEnabled.value,
    gamepad: streamUiInputEnabled.value
  }
})

function resolveRouteViewKey(currentRoute: {
  name?: unknown
  path: string
  fullPath: string
  meta: Record<string, unknown>
}): string {
  if (currentRoute.name === 'xhome-stream' || currentRoute.name === 'xcloud-stream') {
    return currentRoute.fullPath
  }

  return String(currentRoute.name ?? currentRoute.path)
}

function warmupXcloudCatalog(): void {
  if (xcloudWarmupPromise !== undefined) {
    return
  }

  // 提前预热 xCloud 目录缓存，避免首次进入页面时等待完整拉取链路。
  xcloudWarmupPromise = rpc.data
    .getXcloudTitles()
    .catch((error) => {
      console.warn('[App] warmup xcloud catalog failed:', error)
    })
    .finally(() => {
      xcloudWarmupPromise = undefined
    })
}

async function bootstrapAppWarmups(): Promise<void> {
  try {
    const authState = await rpc.auth.getState()
    if (authState.isAuthenticated) {
      warmupXcloudCatalog()
    }
  } catch (error) {
    console.warn('[App] bootstrap warmups failed:', error)
  }
}

onMounted(() => {
  disposeUiDensity = setupUiDensity()
  void bootstrapAppWarmups()
  disposeAuthSessionReady = events.on('auth.sessionReady', () => {
    warmupXcloudCatalog()
  })
  window.addEventListener('stream-ui-input-mode', handleStreamUiInputMode as EventListener)
})

onUnmounted(() => {
  if (disposeUiDensity !== undefined) {
    disposeUiDensity()
    disposeUiDensity = undefined
  }
  if (disposeAuthSessionReady !== undefined) {
    disposeAuthSessionReady()
    disposeAuthSessionReady = undefined
  }
  window.removeEventListener('stream-ui-input-mode', handleStreamUiInputMode as EventListener)
})
</script>

<template>
  <ConsoleUIProvider :input-sources="spatialInputSources" :spatial-navigation="true">
    <SpatialNavGlobalHotkeys v-if="!isStreamRoute || streamUiInputEnabled" />

    <RouterView v-slot="{ Component, route: currentRoute }">
      <AppShellLayout v-if="currentRoute.meta.layout !== 'plain'">
        <KeepAlive>
          <component
            :is="Component"
            v-if="currentRoute.meta.keepAlive"
            :key="resolveRouteViewKey(currentRoute)"
          />
        </KeepAlive>
        <component
          :is="Component"
          v-if="!currentRoute.meta.keepAlive"
          :key="resolveRouteViewKey(currentRoute)"
        />
      </AppShellLayout>

      <template v-else>
        <KeepAlive>
          <component
            :is="Component"
            v-if="currentRoute.meta.keepAlive"
            :key="resolveRouteViewKey(currentRoute)"
          />
        </KeepAlive>
        <component
          :is="Component"
          v-if="!currentRoute.meta.keepAlive"
          :key="resolveRouteViewKey(currentRoute)"
        />
      </template>
    </RouterView>
  </ConsoleUIProvider>
</template>
