<script setup lang="ts">
import type { EventUnsubscribe } from '@shared/events/client'
import type { InputSnapshot } from '@shared/gamepad/input-controller'
import type { AppPageRouteName, TopNavNodeKey } from '../../navigation/spatial-nav.constants'
import { inputController } from '@shared/gamepad/runtime'
import { computed, onMounted, onUnmounted, provide, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { navigationEngine } from '@/navigation/core'
import { playNavSound, triggerNavHaptic } from '@/navigation/core/haptics'
import { requestPageFocus } from '@/navigation/core/usePageFocus'
import { FocusScope } from '@/navigation/core/vue'
import controllerStatusIcon from '../../assets/nav/ctrl.svg'
import myIcon from '../../assets/nav/my.svg'
import settingIcon from '../../assets/nav/setting.svg'
import xboxLogoIcon from '../../assets/nav/xbox-logo.svg'
import xcloudIcon from '../../assets/nav/xcloud.svg'
import xhomeIcon from '../../assets/nav/xhome.svg'
import {

  APP_PAGE_LABEL_KEYS,
  APP_PAGE_NAV_ORDER,
  SPATIAL_NAV_NODE_IDS,
  SPATIAL_NAV_SCOPE_IDS,

} from '../../navigation/spatial-nav.constants'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'
import { devWarn } from '../../shared/dev-log'
import TopNavBar from '../navigation/TopNavBar.vue'
import UserProfileMenu from '../navigation/UserProfileMenu.vue'
import SettingConfirmSheet from '../settings/SettingConfirmSheet.vue'
import { createShellContext, shellContextKey } from './shell-context'
import ShellActionHints from './ShellActionHints.vue'
import { useAuthRecovery } from './use-auth-recovery'

type AuthState = Awaited<ReturnType<typeof rpc.auth.getState>>
type UserProfile = Awaited<ReturnType<typeof rpc.data.getUserProfile>>

const router = useRouter()
const route = useRoute()
const { t } = useI18n()
const authRecovery = useAuthRecovery({ route, router, t })
const shell = createShellContext()
provide(shellContextKey, shell)

const topNavIcons = {
  brand: xboxLogoIcon,
  my: myIcon,
  xhome: xhomeIcon,
  xcloud: xcloudIcon,
  setting: settingIcon,
  controller: controllerStatusIcon,
}

const authState = ref<AuthState | null>(null)
const userProfile = ref<UserProfile | null>(null)
const isProfileMenuOpen = ref(false)
const isLoggingOut = ref(false)
const actionHintsHeight = ref(72)
let disposeAuthSessionReady: EventUnsubscribe | undefined
let disposeGamepadRuntimeSnapshot: EventUnsubscribe | undefined
const gamepadSnapshot = shallowRef<InputSnapshot>(inputController.getSnapshot())

// LB/RB 一级页面切换顺序
const PAGE_NAV_ORDER = APP_PAGE_NAV_ORDER
let disposePageSwitch: (() => void) | undefined

const activeNav = computed<AppPageRouteName>(() => PAGE_NAV_ORDER.includes((route.meta.navPage || route.name) as AppPageRouteName) ? (route.meta.navPage || route.name) as AppPageRouteName : 'xhome')

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
  gamepadSnapshot.value.devices.length > 0,
)
const resolvedStatusText = computed(() => {
  return authState.value?.isAuthenticated ? t('userMenu.loggedIn') : t('userMenu.loggedOut')
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

function closeProfileMenu(): void {
  isProfileMenuOpen.value = false
}

function openProfileMenu(): void {
  isProfileMenuOpen.value = true
}

const canGoBack = computed(() => {
  void route.fullPath
  const back = router.options.history.state.back
  return typeof back === 'string' && back !== route.fullPath && /^\/(?:xhome|xcloud|my|controller|setting)(?:\/|$)/.test(back) && !back.includes('/stream/')
})

function handleEscapeKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape' || event.defaultPrevented) {
    return
  }
  if (isProfileMenuOpen.value || authRecovery.open.value) {
    return // 账户弹窗的焦点与返回由弹窗自身处理。
  }
  if (!shell.overlay.value && canGoBack.value) {
    event.preventDefault()
    router.back()
  }
}

const footerStatus = computed(() => shell.overlay.value?.title || shell.page.value.status || t(APP_PAGE_LABEL_KEYS[activeNav.value]))

watch(actionHintsHeight, () => {
  if (window.matchMedia('(max-width:700px)').matches
    && navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.appShell)
    && navigationEngine.getCurrentFocus()?.closest('.app-shell__content')) {
    navigationEngine.revealCurrentFocus()
  }
}, { flush: 'post' })

async function handleLogout(): Promise<void> {
  if (isLoggingOut.value) {
    return
  }

  isLoggingOut.value = true
  try {
    await rpc.auth.logout()
    navigationEngine.forgetFocus('page:')
    requestPageFocus(false)
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
  requestPageFocus(false, SPATIAL_NAV_NODE_IDS.topNav[node])
  if (node !== 'profile') {
    closeProfileMenu()
  }

  if (node === 'brand' || node === 'xhome') {
    void router.push('/xhome')
    return
  }
  if (node === 'xcloud') {
    void router.push('/xcloud')
    return
  }
  if (node === 'my') {
    void router.push('/my/account')
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
    void router.push('/controller')
  }
}

// --- 触摸滑动切换逻辑 (Touch Swipe Navigation) ---
let touchStartX = 0
let touchStartTime = 0
let touchStartedInRail = false
const SWIPE_THRESHOLD = 80 // 最小滑动位移
const SWIPE_TIMEOUT = 300 // 最大滑动时间

function handleTouchStart(e: TouchEvent): void {
  touchStartedInRail = e.target instanceof Element && !!e.target.closest('.horizontal-list-rail__viewport, input, textarea, select')
  touchStartX = e.touches[0].clientX
  touchStartTime = Date.now()
}

function handleTouchEnd(e: TouchEvent): void {
  if (touchStartedInRail || !navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.appShell))
    return
  const touchEndX = e.changedTouches[0].clientX
  const touchEndTime = Date.now()
  const distance = touchEndX - touchStartX
  const duration = touchEndTime - touchStartTime

  if (duration < SWIPE_TIMEOUT && Math.abs(distance) > SWIPE_THRESHOLD) {
    const navOrder = PAGE_NAV_ORDER
    const currentIndex = navOrder.indexOf(activeNav.value)

    if (distance > 0 && currentIndex > 0) {
      requestPageFocus()
      // 向右划，回退到上一个标签
      void router.push(navOrder[currentIndex - 1] === 'my' ? '/my/account' : { name: navOrder[currentIndex - 1] })
    }
    else if (distance < 0 && currentIndex < navOrder.length - 1) {
      requestPageFocus()
      // 向左划，前进到下一个标签
      void router.push(navOrder[currentIndex + 1] === 'my' ? '/my/account' : { name: navOrder[currentIndex + 1] })
    }
  }
}

watch(
  () => route.fullPath,
  () => {
    closeProfileMenu()
  },
)

watch(authRecovery.open, (open) => {
  if (open)
    closeProfileMenu()
})

onMounted(() => {
  void loadShellUserState()
  disposeAuthSessionReady = events.on('auth.sessionReady', () => {
    void loadShellUserState()
  })
  disposeGamepadRuntimeSnapshot = inputController.subscribe((snapshot) => {
    const previous = gamepadSnapshot.value
    if (previous.owner === snapshot.owner && previous.exclusiveCapture === snapshot.exclusiveCapture
      && previous.devices.length === snapshot.devices.length) {
      return
    }
    gamepadSnapshot.value = snapshot
  })
  window.addEventListener('keydown', handleEscapeKeydown)

  // 注册 LB/RB 一级页面切换
  disposePageSwitch = navigationEngine.onPageSwitch((direction) => {
    if (!navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.appShell) || gamepadSnapshot.value.owner !== 'ui' || gamepadSnapshot.value.exclusiveCapture)
      return
    const currentIndex = PAGE_NAV_ORDER.indexOf(activeNav.value)
    const nextIndex = direction === 'next' ? currentIndex + 1 : currentIndex - 1

    if (nextIndex >= 0 && nextIndex < PAGE_NAV_ORDER.length) {
      requestPageFocus()
      playNavSound('move')
      triggerNavHaptic('move')
      void router.push(PAGE_NAV_ORDER[nextIndex] === 'my' ? '/my/account' : { name: PAGE_NAV_ORDER[nextIndex] })
    }
    else {
      // 已到边界
      playNavSound('boundary')
      triggerNavHaptic('boundary')
    }
  })
})

onUnmounted(() => {
  if (disposeAuthSessionReady !== undefined) {
    disposeAuthSessionReady()
    disposeAuthSessionReady = undefined
  }
  if (disposeGamepadRuntimeSnapshot !== undefined) {
    disposeGamepadRuntimeSnapshot()
    disposeGamepadRuntimeSnapshot = undefined
  }
  window.removeEventListener('keydown', handleEscapeKeydown)
  if (disposePageSwitch !== undefined) {
    disposePageSwitch()
    disposePageSwitch = undefined
  }
})
</script>

<template>
  <section class="app-shell" :style="{ '--ui-shell-floating-hints-height': `${actionHintsHeight}px` }">
    <FocusScope
      :id="SPATIAL_NAV_SCOPE_IDS.appShell"
      :default-focus-id="SPATIAL_NAV_NODE_IDS.topNav.brand"
    >
      <TopNavBar
        :icons="topNavIcons"
        :active-nav="activeNav"
        :profile-image-url="resolvedAvatarUrl"
        :profile-display-name="resolvedDisplayName"
        :profile-open="isProfileMenuOpen"
        :controller-active="hasConnectedGamepad"
        @select="handleTopNavSelect"
      />

      <main
        class="app-shell__content"
        data-nav-scroll-container
        :inert="isProfileMenuOpen"
        @touchstart="handleTouchStart"
        @touchend="handleTouchEnd"
      >
        <slot />
      </main>
      <ShellActionHints
        :region-label="shell.page.value.regionLabel"
        :status="footerStatus"
        :error="!shell.overlay.value && shell.page.value.error"
        :overlay="!!shell.overlay.value"
        :back="canGoBack"
        :busy="shell.overlay.value?.busy"
        :available="gamepadSnapshot.owner === 'ui'"
        :exclusive="gamepadSnapshot.exclusiveCapture"
        @resize="actionHintsHeight = $event"
      />
    </FocusScope>

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
      @my="handleTopNavSelect('my')"
    />
    <SettingConfirmSheet
      :open="authRecovery.open.value"
      scope-id="auth.recovery"
      :title="t('streamPage.failure.login')"
      :message="authRecovery.message.value"
      :confirm-label="t('streamPage.failure.login')"
      :busy="authRecovery.busy.value"
      @close="authRecovery.cancel"
      @confirm="authRecovery.confirm"
    />
  </section>
</template>

<style scoped>
.app-shell {
  position: relative;
  display: grid;
  grid-template-columns: var(--ui-shell-dock-width) minmax(0, 1fr);
  grid-template-rows: var(--ui-shell-header-height) minmax(0, 1fr) auto;
  column-gap: var(--ui-shell-gap);
  width: 100%;
  min-width: 0;
  height: 100dvh;
  color: var(--ui-page-text);
  font-family: var(--ui-font-family);
  --ui-shell-content-top: 16px;
  background-color: var(--ui-page-bg);
  padding: 0 var(--ui-shell-edge-inset) 0 var(--space-12);
  overflow: hidden;
}

.app-shell::before { content: ''; position: absolute; inset: 0; z-index: 0; pointer-events: none; background: radial-gradient(ellipse 52% 70% at 48% 0%, color-mix(in srgb, var(--ui-ambient-color) 60%, transparent), transparent); }

.app-shell__content {
  position: relative;
  z-index: 1;
  grid-column: 2;
  grid-row: 2;
  min-width: 0;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding: var(--ui-shell-content-top) var(--ui-focus-safe-inset) var(--space-8);
  scroll-padding: var(--ui-focus-safe-inset) var(--ui-focus-safe-inset) var(--space-16);
  /* 横向游戏轨道与页面纵向滚动共用浏览器原生触摸滚动。 */
  touch-action: pan-x pan-y;
}

@media(max-width:1100px), (max-height:680px) { .app-shell { --ui-shell-content-top: 12px; --ui-shell-gap: 16px; --ui-shell-edge-inset: 16px; --ui-console-hero-height: 208px; --ui-console-hero-padding: 16px; --ui-console-hero-image-height: 160px; --ui-console-hero-title-size: 24px; } }
@media(max-width:700px) {
  .app-shell {
    --ui-shell-bottom-nav-height: calc(70px + env(safe-area-inset-bottom, 0px));
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: var(--ui-shell-header-height) minmax(0, 1fr) var(--ui-shell-bottom-nav-height);
    padding-inline: 0;
  }
  /* 正文延伸至毛玻璃导航后方，焦点为导航和提示留出完整空间。 */
  .app-shell__content {
    grid-column: 1;
    grid-row: 2 / 4;
    padding: var(--space-12) var(--space-12) calc(var(--ui-shell-bottom-nav-height) + var(--ui-shell-floating-hints-height) + var(--space-16));
    scroll-padding-bottom: calc(var(--ui-shell-bottom-nav-height) + var(--ui-shell-floating-hints-height) + var(--space-16));
  }
}
@media(max-width:600px) {
  /* 二级菜单与内容上下分栏，共用这套骨架的设置、控制器和资料页一起适配。 */
  .app-shell {
    --ui-settings-layout-columns: minmax(0, 1fr);
    --ui-settings-layout-rows: auto minmax(0, 1fr);
    --ui-settings-sidebar-header-display: none;
    --ui-settings-sidebar-nav-direction: row;
    --ui-settings-sidebar-tab-width: auto;
    --ui-settings-sidebar-gap: 0px;
    --ui-settings-side-padding: var(--space-12);
  }
  .app-shell__content {
    padding-inline: var(--space-12);
    padding-bottom: calc(var(--ui-shell-bottom-nav-height) + var(--space-16));
    scroll-padding-bottom: calc(var(--ui-shell-bottom-nav-height) + var(--space-16));
  }
}
</style>
