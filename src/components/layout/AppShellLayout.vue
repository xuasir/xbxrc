<script setup lang="ts">
import type { EventUnsubscribe } from '@shared/events/client'
import type { GamepadRuntimeSnapshotDto } from '@shared/gamepad/contract'
import type { AppPageRouteName, TopNavNodeKey } from '../../navigation/spatial-nav.constants'
import { businessInputArbiter, toBusinessInputTracePayload } from '@shared/gamepad/business-input-arbiter'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { navigationEngine } from '@/navigation/core'
import { playNavSound, triggerNavHaptic } from '@/navigation/core/haptics'
import { FocusScope } from '@/navigation/core/vue'
import controllerStatusIcon from '../../assets/nav/ctrl.svg'
import settingIcon from '../../assets/nav/setting.svg'
import xboxLogoIcon from '../../assets/nav/xbox-logo.svg'
import xcloudIcon from '../../assets/nav/xcloud.svg'
import xhomeIcon from '../../assets/nav/xhome.svg'
import {

  SPATIAL_NAV_NODE_IDS,
  SPATIAL_NAV_SCOPE_IDS,

} from '../../navigation/spatial-nav.constants'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'
import { devWarn, devWarnRateLimited } from '../../shared/dev-log'
import GamepadProfileCard from '../navigation/GamepadProfileCard.vue'
import TopNavBar from '../navigation/TopNavBar.vue'
import UserProfileMenu from '../navigation/UserProfileMenu.vue'

type AuthState = Awaited<ReturnType<typeof rpc.auth.getState>>
type UserProfile = Awaited<ReturnType<typeof rpc.data.getUserProfile>>

const router = useRouter()
const route = useRoute()
const { t } = useI18n()

const topNavIcons = {
  brand: xboxLogoIcon,
  xhome: xhomeIcon,
  xcloud: xcloudIcon,
  setting: settingIcon,
  controller: controllerStatusIcon,
}

const authState = ref<AuthState | null>(null)
const userProfile = ref<UserProfile | null>(null)
const isProfileMenuOpen = ref(false)
const isGamepadCardOpen = ref(false)
const isLoggingOut = ref(false)
let disposeAuthSessionReady: EventUnsubscribe | undefined
let disposeGamepadRuntimeSnapshot: EventUnsubscribe | undefined
const gamepadSnapshot = ref<GamepadRuntimeSnapshotDto | null>(null)
let lastSnapshotTraceSignature: string | null = null
let shellFocusRepairFrame: number | null = null

// LB/RB 一级页面切换顺序
const PAGE_NAV_ORDER: AppPageRouteName[] = ['xhome', 'xcloud', 'setting']
let disposePageSwitch: (() => void) | undefined

const activeNav = computed<'xhome' | 'xcloud' | 'setting'>(() => {
  if (route.name === 'xcloud') {
    return 'xcloud'
  }
  if (route.name === 'setting') {
    return 'setting'
  }
  return 'xhome'
})

const resolvedDisplayName = computed(() => {
  return (
    userProfile.value?.gameDisplayName
    || userProfile.value?.gamertag
    || t('userMenu.unknownUser')
  )
})

const resolvedSecondaryName = computed(() => {
  if (!userProfile.value?.gamertag || userProfile.value.gamertag === resolvedDisplayName.value) {
    return ''
  }
  return `@${userProfile.value.gamertag}`
})

const resolvedScore = computed(() => userProfile.value?.gamerscore || '0')
const resolvedAvatarUrl = computed(() => userProfile.value?.gameDisplayPicRaw || '')
const hasConnectedGamepad = computed(() =>
  (gamepadSnapshot.value?.devices ?? []).some(device => device.connected),
)
const resolvedStatusText = computed(() => {
  return authState.value?.isAuthenticated ? t('userMenu.loggedIn') : t('userMenu.loggedOut')
})

watch(hasConnectedGamepad, (connected) => {
  if (!connected && isGamepadCardOpen.value) {
    // 控制器断开时避免卡片悬空展示
    closeGamepadCard()
  }
})

async function loadShellUserState(): Promise<void> {
  try {
    const [nextAuthState, nextUserProfile] = await Promise.all([
      rpc.auth.getState(),
      rpc.data.getUserProfile(),
    ])
    authState.value = nextAuthState
    userProfile.value = nextUserProfile
  }
  catch (error) {
    devWarn('[AppShell] load user state failed:', error)
  }
}

async function loadGamepadSnapshot(): Promise<void> {
  try {
    gamepadSnapshot.value = await rpc.gamepad.getRuntimeSnapshot()
    traceSnapshotTransition('loadGamepadSnapshot', gamepadSnapshot.value)
  }
  catch (error) {
    recordGamepadRecoveryTrace('gamepadRuntimeSnapshotLoadFailed', {
      source: 'loadGamepadSnapshot',
      error: error instanceof Error ? error.message : String(error),
      documentVisibility: document.visibilityState,
      hasFocus: document.hasFocus(),
    })
    devWarnRateLimited('app-shell:load-gamepad-snapshot-failed', '[AppShell] load gamepad snapshot failed:', error)
  }
}

function recordGamepadRecoveryTrace(event: string, payload: Record<string, unknown>): void {
  void rpc.runtimeTrace.recordEvent({
    event,
    payload,
  }).catch(() => {})
}

function connectedDeviceCount(snapshot: GamepadRuntimeSnapshotDto | null): number {
  return snapshot?.devices.filter(device => device.connected).length ?? 0
}

function resolveSnapshotTracePayload(snapshot: GamepadRuntimeSnapshotDto | null): Record<string, unknown> {
  const businessInputTrace = toBusinessInputTracePayload({
    state: businessInputArbiter.getState(),
    owner: businessInputArbiter.getOwner(),
  })
  if (!snapshot) {
    return {
      hasSnapshot: false,
      documentVisibility: document.visibilityState,
      hasFocus: document.hasFocus(),
      ...businessInputTrace,
    }
  }

  let maxSampleSeq = -1
  let maxSampledAtMs = -1
  for (const slot of snapshot.slots) {
    maxSampleSeq = Math.max(maxSampleSeq, slot.sampleSeq)
    maxSampledAtMs = Math.max(maxSampledAtMs, slot.sampledAtMs)
  }

  return {
    hasSnapshot: true,
    streamPadForwarding: snapshot.streamPadForwarding ?? false,
    samplingLifecycle: snapshot.samplingLifecycle ?? 'active',
    samplingHealth: snapshot.samplingHealth ?? 'healthy',
    connectedDevices: connectedDeviceCount(snapshot),
    slotCount: snapshot.slots.length,
    maxSampleSeq,
    maxSampledAtMs,
    lastSampleProgressAtMs: snapshot.lastSampleProgressAtMs ?? null,
    lastBackendSampleActivityAtMs: snapshot.lastBackendSampleActivityAtMs ?? null,
    samplingSelfHealCount: snapshot.samplingSelfHealCount ?? null,
    documentVisibility: document.visibilityState,
    hasFocus: document.hasFocus(),
    ...businessInputTrace,
  }
}

function traceSnapshotTransition(source: string, snapshot: GamepadRuntimeSnapshotDto | null): void {
  const payload = resolveSnapshotTracePayload(snapshot)
  const signature = JSON.stringify([
    source,
    payload.streamPadForwarding ?? false,
    payload.samplingLifecycle ?? null,
    payload.samplingHealth ?? null,
    payload.connectedDevices ?? 0,
    payload.slotCount ?? 0,
    payload.maxSampleSeq ?? null,
    payload.maxSampledAtMs ?? null,
    payload.lastSampleProgressAtMs ?? null,
    payload.lastBackendSampleActivityAtMs ?? null,
    payload.samplingSelfHealCount ?? null,
    payload.documentVisibility,
    payload.hasFocus,
  ])
  if (signature === lastSnapshotTraceSignature) {
    return
  }
  lastSnapshotTraceSignature = signature
  recordGamepadRecoveryTrace('gamepadRuntimeSnapshotObserved', {
    source,
    ...payload,
  })
}

function handleDocumentVisibilityChange(): void {
  recordGamepadRecoveryTrace('gamepadDocumentVisibilityChanged', {
    visibilityState: document.visibilityState,
    hasFocus: document.hasFocus(),
    snapshot: resolveSnapshotTracePayload(gamepadSnapshot.value),
  })
  if (document.visibilityState !== 'visible') {
    return
  }
  void ensureShellFocusReady('document-visible')
}

function clearShellFocusRepairFrame(): void {
  if (shellFocusRepairFrame !== null) {
    window.cancelAnimationFrame(shellFocusRepairFrame)
    shellFocusRepairFrame = null
  }
}

async function ensureShellFocusReady(reason: string): Promise<void> {
  if (isProfileMenuOpen.value || isGamepadCardOpen.value) {
    return
  }
  if (document.querySelector('[data-focused="true"]') instanceof HTMLElement) {
    return
  }

  await nextTick()
  clearShellFocusRepairFrame()
  shellFocusRepairFrame = window.requestAnimationFrame(() => {
    shellFocusRepairFrame = null
    if (document.querySelector('[data-focused="true"]') instanceof HTMLElement) {
      return
    }
    const defaultFocus = document.getElementById(SPATIAL_NAV_NODE_IDS.topNav.brand)
    if (defaultFocus instanceof HTMLElement) {
      navigationEngine.focusElement(defaultFocus, false)
      recordGamepadRecoveryTrace('gamepadShellFocusRepaired', {
        reason,
        targetId: SPATIAL_NAV_NODE_IDS.topNav.brand,
        visibilityState: document.visibilityState,
        hasFocus: document.hasFocus(),
      })
    }
  })
}

function handleWindowFocus(): void {
  recordGamepadRecoveryTrace('gamepadWindowFocused', {
    visibilityState: document.visibilityState,
    hasFocus: document.hasFocus(),
    snapshot: resolveSnapshotTracePayload(gamepadSnapshot.value),
  })
  void ensureShellFocusReady('window-focus')
}

function closeProfileMenu(): void {
  isProfileMenuOpen.value = false
}

function openProfileMenu(): void {
  closeGamepadCard()
  isProfileMenuOpen.value = true
}

function closeGamepadCard(): void {
  isGamepadCardOpen.value = false
}

function openGamepadCard(): void {
  closeProfileMenu()
  isGamepadCardOpen.value = true
}

function handleEscapeKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape') {
    return
  }
  if (isProfileMenuOpen.value) {
    closeProfileMenu()
  }
  if (isGamepadCardOpen.value) {
    closeGamepadCard()
  }
}

async function handleLogout(): Promise<void> {
  if (isLoggingOut.value) {
    return
  }

  isLoggingOut.value = true
  try {
    await rpc.auth.logout()
    authState.value = {
      provider: authState.value?.provider ?? 'xal',
      isAuthenticating: false,
      isAuthenticated: false,
      appLevel: 0,
    }
    closeProfileMenu()
    await router.replace('/login')
  }
  catch (error) {
    console.error('[AppShell] logout failed:', error)
  }
  finally {
    isLoggingOut.value = false
  }
}

function handleTopNavSelect(node: TopNavNodeKey): void {
  if (node !== 'profile') {
    closeProfileMenu()
  }
  if (node !== 'controller') {
    closeGamepadCard()
  }

  if (node === 'brand' || node === 'xhome') {
    void router.push('/xhome')
    return
  }
  if (node === 'xcloud') {
    void router.push('/xcloud')
    return
  }
  if (node === 'setting') {
    void router.push('/setting')
    return
  }
  if (node === 'profile') {
    openProfileMenu()
    return
  }
  if (node === 'controller') {
    openGamepadCard()
  }
}

// --- 触摸滑动切换逻辑 (Touch Swipe Navigation) ---
let touchStartX = 0
let touchStartTime = 0
const SWIPE_THRESHOLD = 80 // 最小滑动位移
const SWIPE_TIMEOUT = 300 // 最大滑动时间

function handleTouchStart(e: TouchEvent): void {
  touchStartX = e.touches[0].clientX
  touchStartTime = Date.now()
}

function handleTouchEnd(e: TouchEvent): void {
  const touchEndX = e.changedTouches[0].clientX
  const touchEndTime = Date.now()
  const distance = touchEndX - touchStartX
  const duration = touchEndTime - touchStartTime

  if (duration < SWIPE_TIMEOUT && Math.abs(distance) > SWIPE_THRESHOLD) {
    const navOrder: AppPageRouteName[] = ['xhome', 'xcloud', 'setting']
    const currentIndex = navOrder.indexOf(activeNav.value as any)

    if (distance > 0 && currentIndex > 0) {
      // 向右划，回退到上一个标签
      void router.push({ name: navOrder[currentIndex - 1] })
    }
    else if (distance < 0 && currentIndex < navOrder.length - 1) {
      // 向左划，前进到下一个标签
      void router.push({ name: navOrder[currentIndex + 1] })
    }
  }
}

watch(
  () => route.fullPath,
  () => {
    closeProfileMenu()
    closeGamepadCard()
  },
)

onMounted(() => {
  recordGamepadRecoveryTrace('gamepadShellMounted', {
    routeName: String(route.name ?? ''),
    routePath: route.fullPath,
    visibilityState: document.visibilityState,
    hasFocus: document.hasFocus(),
  })
  void loadShellUserState()
  void loadGamepadSnapshot()
  disposeAuthSessionReady = events.on('auth.sessionReady', () => {
    void loadShellUserState()
  })
  disposeGamepadRuntimeSnapshot = events.on('gamepad.runtimeSnapshot', (snapshot) => {
    gamepadSnapshot.value = snapshot
    traceSnapshotTransition('runtimeSnapshotEvent', snapshot)
  })
  window.addEventListener('keydown', handleEscapeKeydown)
  document.addEventListener('visibilitychange', handleDocumentVisibilityChange)
  window.addEventListener('focus', handleWindowFocus)
  void ensureShellFocusReady('app-shell-mounted')

  // 注册 LB/RB 一级页面切换
  disposePageSwitch = navigationEngine.onPageSwitch((direction) => {
    const currentIndex = PAGE_NAV_ORDER.indexOf(activeNav.value)
    const nextIndex = direction === 'next' ? currentIndex + 1 : currentIndex - 1

    if (nextIndex >= 0 && nextIndex < PAGE_NAV_ORDER.length) {
      playNavSound('move')
      triggerNavHaptic('move')
      void router.push({ name: PAGE_NAV_ORDER[nextIndex] })
    }
    else {
      // 已到边界
      playNavSound('boundary')
      triggerNavHaptic('boundary')
    }
  })
})

onUnmounted(() => {
  recordGamepadRecoveryTrace('gamepadShellUnmounted', {
    routeName: String(route.name ?? ''),
    routePath: route.fullPath,
    visibilityState: document.visibilityState,
    hasFocus: document.hasFocus(),
    snapshot: resolveSnapshotTracePayload(gamepadSnapshot.value),
  })
  if (disposeAuthSessionReady !== undefined) {
    disposeAuthSessionReady()
    disposeAuthSessionReady = undefined
  }
  if (disposeGamepadRuntimeSnapshot !== undefined) {
    disposeGamepadRuntimeSnapshot()
    disposeGamepadRuntimeSnapshot = undefined
  }
  window.removeEventListener('keydown', handleEscapeKeydown)
  document.removeEventListener('visibilitychange', handleDocumentVisibilityChange)
  window.removeEventListener('focus', handleWindowFocus)
  clearShellFocusRepairFrame()
  if (disposePageSwitch !== undefined) {
    disposePageSwitch()
    disposePageSwitch = undefined
  }
})
</script>

<template>
  <section class="app-shell">
    <FocusScope
      :id="SPATIAL_NAV_SCOPE_IDS.appShell"
      :default-focus-id="SPATIAL_NAV_NODE_IDS.topNav.brand"
      :active="!isProfileMenuOpen"
    >
      <TopNavBar
        :icons="topNavIcons"
        :active-nav="activeNav"
        :profile-image-url="resolvedAvatarUrl"
        :controller-active="isGamepadCardOpen || hasConnectedGamepad"
        :show-controller="hasConnectedGamepad"
        @select="handleTopNavSelect"
      />

      <main
        class="app-shell__content"
        @touchstart="handleTouchStart"
        @touchend="handleTouchEnd"
      >
        <slot />
      </main>
    </FocusScope>

    <GamepadProfileCard
      :open="isGamepadCardOpen"
      :snapshot="gamepadSnapshot"
      @close="closeGamepadCard"
    />

    <UserProfileMenu
      :open="isProfileMenuOpen"
      :display-name="resolvedDisplayName"
      :secondary-name="resolvedSecondaryName"
      :score="resolvedScore"
      :status-text="resolvedStatusText"
      :avatar-url="resolvedAvatarUrl"
      :logging-out="isLoggingOut"
      @close="closeProfileMenu"
      @logout="() => void handleLogout()"
    />
  </section>
</template>

<style scoped>
.app-shell {
  position: relative;
  display: flex;
  flex-direction: column;
  width: 100%;
  min-width: 0;
  min-height: 100vh;
  height: 100vh;
  color: var(--ui-page-text);
  font-family: var(--ui-font-family);
  background-color: var(--ui-page-bg);
  overflow: hidden;
}

.app-shell__content {
  position: relative;
  z-index: 1;
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  overflow-x: visible;
  padding: var(--ui-page-inset);
  padding-top: var(--ui-app-shell-content-padding-top);
  /* 允许垂直滚动，但让出水平滑动权限给我们的 JS 逻辑 */
  touch-action: pan-y;
}
</style>
