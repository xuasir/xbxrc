<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { setupTheme } from './app/theme'
import { setupUiDensity } from './app/ui-density'
import AppShellLayout from './components/layout/AppShellLayout.vue'
import SpatialNavGlobalHotkeys from './components/navigation/SpatialNavGlobalHotkeys.vue'
import { useGamepadNavigation } from './navigation/core'
import { events } from './services/events'
import { rpc } from './services/rpc'
import { devWarn } from './shared/dev-log'

const route = useRoute()
let disposeAuthSessionReady: (() => void) | undefined
let disposeUiDensity: (() => void) | undefined
let xcloudWarmupPromise: Promise<unknown> | undefined
const streamUiInputEnabled = ref(false)

useGamepadNavigation()

function handleStreamUiInputMode(event: Event): void {
  const detail = (event as CustomEvent<{ enabled?: boolean }>).detail
  streamUiInputEnabled.value = detail?.enabled === true
}

const isStreamRoute = computed(
  () => route.name === 'xhome-stream' || route.name === 'xcloud-stream',
)

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

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

  xcloudWarmupPromise = rpc.data
    .primeXcloudTitles()
    .catch((error) => {
      devWarn('[App] warmup xcloud catalog failed:', error)
    })
    .finally(() => {
      xcloudWarmupPromise = undefined
    })
}

async function bootstrapAppWarmups(): Promise<void> {
  try {
    const config = await rpc.config.get({ keys: ['theme'] })
    if (isRecord(config) && typeof config.theme === 'string') {
      setupTheme(config.theme as any)
    }
    else {
      setupTheme('dark')
    }

    const authState = await rpc.auth.getState()
    if (authState.isAuthenticated) {
      warmupXcloudCatalog()
    }
  }
  catch (error) {
    devWarn('[App] bootstrap warmups failed:', error)
  }
}

onMounted(() => {
  disposeUiDensity = setupUiDensity()
  void bootstrapAppWarmups()
  disposeAuthSessionReady = events.on('auth.sessionReady', () => {
    warmupXcloudCatalog()
  })
  window.addEventListener('stream-ui-input-mode', handleStreamUiInputMode)
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
  window.removeEventListener('stream-ui-input-mode', handleStreamUiInputMode)
})
</script>

<template>
  <div class="app-root" data-nav-zone="app-root" style="display: contents">
    <SpatialNavGlobalHotkeys v-if="!isStreamRoute || streamUiInputEnabled" />

    <RouterView v-slot="{ Component, route: currentRoute }">
      <!--
        为了保持头部导航稳定，不再将 AppShellLayout 置于 Transition 内部。
        只有内容区域进行切换动画。
      -->
      <AppShellLayout v-if="currentRoute.meta.layout === 'shell'">
        <Transition name="page-fade" mode="out-in">
          <KeepAlive>
            <component
              :is="Component"
              :key="currentRoute.name"
            />
          </KeepAlive>
        </Transition>
      </AppShellLayout>

      <div
        v-else
        class="app-view-plain"
        :class="{ 'app-view-plain--stream': isStreamRoute }"
      >
        <component
          :is="Component"
          v-if="isStreamRoute"
          :key="resolveRouteViewKey(currentRoute)"
        />

        <Transition v-else name="page-fade" mode="out-in">
          <component
            :is="Component"
            :key="resolveRouteViewKey(currentRoute)"
          />
        </Transition>
      </div>
    </RouterView>
  </div>
</template>
