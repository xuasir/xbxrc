<script setup lang="ts">
import type { EventUnsubscribe } from '@shared/events/client'
import { FocusScope } from '@spatial-navigation/vue'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import controllerStatusIcon from '../../assets/nav/no-ctrl.svg'
import settingIcon from '../../assets/nav/setting.svg'
import mainBgImage from '../../assets/main-bg.jpg'
import xboxLogoIcon from '../../assets/nav/xbox-logo.svg'
import xcloudIcon from '../../assets/nav/xcloud.svg'
import xhomeIcon from '../../assets/nav/xhome.svg'
import {
  SPATIAL_NAV_NODE_IDS,
  SPATIAL_NAV_SCOPE_IDS,
  type AppPageRouteName,
  type TopNavNodeKey
} from '../../navigation/spatial-nav.constants'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'
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
  controller: controllerStatusIcon
}

const authState = ref<AuthState | null>(null)
const userProfile = ref<UserProfile | null>(null)
const isProfileMenuOpen = ref(false)
const isLoggingOut = ref(false)
let disposeAuthSessionReady: EventUnsubscribe | undefined

const activeNav = computed<'xhome' | 'xcloud' | 'setting'>(() => {
  if (route.name === 'xcloud') {
    return 'xcloud'
  }
  if (route.name === 'setting') {
    return 'setting'
  }
  return 'xhome'
})

const currentPageFocusNodeId = computed<string | undefined>(() => {
  const routeName = route.name
  if (routeName !== 'xhome' && routeName !== 'xcloud' && routeName !== 'setting') {
    return undefined
  }
  return SPATIAL_NAV_NODE_IDS.pagePrimary[routeName as AppPageRouteName]
})

const profileDownNeighborId = computed<string | undefined>(() => {
  return isProfileMenuOpen.value ? SPATIAL_NAV_NODE_IDS.userMenu.logout : currentPageFocusNodeId.value
})

// 通过 CSS 变量注入背景图，方便后续主题或皮肤替换
const shellStyleVars = computed(() => ({
  '--app-shell-bg-image': `url(${mainBgImage})`
}))

const resolvedDisplayName = computed(() => {
  return (
    userProfile.value?.gameDisplayName ||
    userProfile.value?.gamertag ||
    t('userMenu.unknownUser')
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
const resolvedStatusText = computed(() => {
  return authState.value?.isAuthenticated ? t('userMenu.loggedIn') : t('userMenu.loggedOut')
})

async function loadShellUserState(): Promise<void> {
  try {
    const [nextAuthState, nextUserProfile] = await Promise.all([
      rpc.auth.getState(),
      rpc.data.getUserProfile()
    ])
    authState.value = nextAuthState
    userProfile.value = nextUserProfile
  } catch (error) {
    console.warn('[AppShell] load user state failed:', error)
  }
}

function closeProfileMenu(): void {
  isProfileMenuOpen.value = false
}

function openProfileMenu(): void {
  isProfileMenuOpen.value = true
}

function handleEscapeKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape' || !isProfileMenuOpen.value) {
    return
  }
  closeProfileMenu()
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
      appLevel: 0
    }
    closeProfileMenu()
    await router.replace('/login')
  } catch (error) {
    console.error('[AppShell] logout failed:', error)
  } finally {
    isLoggingOut.value = false
  }
}

function handleTopNavSelect(node: TopNavNodeKey): void {
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
  if (node === 'setting') {
    void router.push('/setting')
    return
  }
  if (node === 'profile') {
    openProfileMenu()
  }
}

watch(
  () => route.fullPath,
  () => {
    closeProfileMenu()
  }
)

onMounted(() => {
  void loadShellUserState()
  disposeAuthSessionReady = events.on('auth.sessionReady', () => {
    void loadShellUserState()
  })
  window.addEventListener('keydown', handleEscapeKeydown)
})

onUnmounted(() => {
  if (disposeAuthSessionReady !== undefined) {
    disposeAuthSessionReady()
    disposeAuthSessionReady = undefined
  }
  window.removeEventListener('keydown', handleEscapeKeydown)
})
</script>

<template>
  <section class="app-shell" :style="shellStyleVars">
    <FocusScope
      :id="SPATIAL_NAV_SCOPE_IDS.appShell"
      :default-focus-id="SPATIAL_NAV_NODE_IDS.topNav.brand"
      :active="!isProfileMenuOpen"
    >
      <div class="app-shell__bg-glow" aria-hidden="true"></div>

      <TopNavBar
        :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
        :down-neighbor-id="currentPageFocusNodeId"
        :profile-down-neighbor-id="profileDownNeighborId"
        :icons="topNavIcons"
        :active-nav="activeNav"
        :profile-image-url="resolvedAvatarUrl"
        @select="handleTopNavSelect"
      />

      <main class="app-shell__content">
        <slot />
      </main>
    </FocusScope>

    <UserProfileMenu
      :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
      :logout-up-neighbor-id="SPATIAL_NAV_NODE_IDS.topNav.profile"
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
  background-color: #06110d;
  background-image:
    linear-gradient(180deg, rgba(0, 0, 0, 0.44), rgba(0, 0, 0, 0.72)), var(--app-shell-bg-image);
  background-size: cover;
  background-position: center;
  background-repeat: no-repeat;
  overflow: hidden;
}

.app-shell__bg-glow {
  position: absolute;
  inset: 0;
  background:
    radial-gradient(circle at 50% 50%, var(--ui-page-glow-soft), transparent 48%),
    radial-gradient(circle at 50% 20%, var(--ui-page-glow-strong), transparent 32%);
  pointer-events: none;
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
}
</style>
