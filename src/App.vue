<script setup lang="ts">
import type { RuntimeConfig } from '@spatial-navigation/runtime'
import { ConsoleUIProvider } from '@spatial-navigation/vue'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { setupUiDensity } from './app/ui-density'
import AppShellLayout from './components/layout/AppShellLayout.vue'
import SpatialNavGlobalHotkeys from './components/navigation/SpatialNavGlobalHotkeys.vue'
import { SPATIAL_NAV_RUNTIME_EVENTS } from './navigation/spatial-nav.constants'
import { events } from './services/events'
import { rpc } from './services/rpc'

const route = useRoute()
let disposeAuthSessionReady: (() => void) | undefined
let disposeUiDensity: (() => void) | undefined
let xcloudWarmupPromise: Promise<unknown> | undefined
const streamUiInputEnabled = ref(false)
const STREAM_GAMEPLAY_GUARD_KEYS = new Set([
  'ArrowUp',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'Tab',
  'Enter',
  ' ',
])

function handleStreamUiInputMode(event: Event): void {
  const detail = (event as CustomEvent<{ enabled?: boolean }>).detail
  streamUiInputEnabled.value = detail?.enabled === true
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false
  }
  return (
    target instanceof HTMLInputElement
    || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement
    || target.isContentEditable
    || target.closest('[contenteditable="true"]') !== null
  )
}

function handleStreamGameplayKeyboardCapture(event: KeyboardEvent): void {
  if (!isStreamRoute.value || streamUiInputEnabled.value) {
    return
  }
  if (event.altKey || event.ctrlKey || event.metaKey) {
    return
  }
  if (!STREAM_GAMEPLAY_GUARD_KEYS.has(event.key) || isEditableTarget(event.target)) {
    return
  }

  // stream gameplay 态下不让方向键/确认键再落入 spatial navigation。
  event.preventDefault()
  event.stopPropagation()
}

const isStreamRoute = computed(
  () => route.name === 'xhome-stream' || route.name === 'xcloud-stream',
)

const spatialInputSources = computed(() => {
  if (!isStreamRoute.value) {
    return { keyboard: true, gamepad: true }
  }
  return {
    keyboard: streamUiInputEnabled.value,
    gamepad: streamUiInputEnabled.value,
  }
})

const spatialRuntimeConfig: RuntimeConfig = {
  onDebugEvent(event) {
    if (event.type !== 'action' || !event.detail.startsWith('TAB_NAV:')) {
      return
    }
    if (typeof window === 'undefined') {
      return
    }

    // 统一广播 TAB_NAV 动作，保证键盘与手柄来源都能触发后续联动逻辑。
    window.dispatchEvent(new CustomEvent(SPATIAL_NAV_RUNTIME_EVENTS.tabNavAction))
  },
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
  }
  catch (error) {
    console.warn('[App] bootstrap warmups failed:', error)
  }
}

onMounted(() => {
  disposeUiDensity = setupUiDensity()
  void bootstrapAppWarmups()
  disposeAuthSessionReady = events.on('auth.sessionReady', () => {
    warmupXcloudCatalog()
  })
  window.addEventListener('stream-ui-input-mode', handleStreamUiInputMode)
  window.addEventListener('keydown', handleStreamGameplayKeyboardCapture, { capture: true })
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
  window.removeEventListener('keydown', handleStreamGameplayKeyboardCapture, { capture: true })
})
</script>

<template>
  <ConsoleUIProvider
    :config="spatialRuntimeConfig"
    :input-sources="spatialInputSources"
    :spatial-navigation="true"
  >
    <SpatialNavGlobalHotkeys v-if="!isStreamRoute || streamUiInputEnabled" />

    <RouterView v-slot="{ Component, route: currentRoute }">
      <Transition name="page-fade" mode="out-in">
        <div :key="resolveRouteViewKey(currentRoute)" class="app-view-container">
          <AppShellLayout v-if="currentRoute.meta.layout !== 'plain'">
            <KeepAlive>
              <component
                :is="Component"
                v-if="currentRoute.meta.keepAlive"
              />
            </KeepAlive>
            <component
              :is="Component"
              v-if="!currentRoute.meta.keepAlive"
            />
          </AppShellLayout>

          <div v-else class="app-view-plain">
            <KeepAlive>
              <component
                :is="Component"
                v-if="currentRoute.meta.keepAlive"
              />
            </KeepAlive>
            <component
              :is="Component"
              v-if="!currentRoute.meta.keepAlive"
            />
          </div>
        </div>
      </Transition>
    </RouterView>
  </ConsoleUIProvider>
</template>
