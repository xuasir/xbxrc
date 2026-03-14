<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { events } from '../services/events'
import { rpc } from '../services/rpc'

const { t } = useI18n()
const router = useRouter()
const route = useRoute()

type LoginViewState = 'checking' | 'idle' | 'submitting'

const viewState = ref<LoginViewState>('checking')

let disposeAuthSessionReady: (() => void) | undefined
let disposeAuthStateChanged: (() => void) | undefined

const redirectTarget = computed(() => {
  return typeof route.query.redirect === 'string' ? route.query.redirect : '/xhome'
})

const isLoginSubmitting = computed(() => viewState.value === 'submitting')
const isLoading = computed(() => viewState.value !== 'idle')
const loginStatusLine = computed(() => {
  if (viewState.value === 'checking') {
    return t('login.checkingStatusLine')
  }
  if (viewState.value === 'submitting') {
    return t('login.signingStatusLine')
  }
  return t('login.playLine')
})
const loginActionLabel = computed(() => {
  if (viewState.value === 'checking') {
    return t('login.restoringSession')
  }
  if (viewState.value === 'submitting') {
    return t('login.signingIn')
  }
  return t('login.signInSimple')
})

async function redirectIfAuthenticated(): Promise<boolean> {
  const authState = await rpc.auth.getState()
  if (authState.isAuthenticated) {
    await router.replace(redirectTarget.value)
    return true
  }
  return false
}

function applyAuthStateView(
  authState: Awaited<ReturnType<typeof rpc.auth.getState>>,
  options: {
    preserveSubmitting?: boolean
  } = {},
): void {
  if (authState.isAuthenticated) {
    // 成功后由 redirectIfAuthenticated 处理
    return
  }

  if (authState.isAuthenticating) {
    // 显式登录期间继续保持“正在登录”语义，避免误显示成“恢复登录”
    viewState.value
      = options.preserveSubmitting && viewState.value === 'submitting' ? 'submitting' : 'checking'
    return
  }

  viewState.value = 'idle'
}

async function bootstrapLoginState(): Promise<void> {
  const currentState = await rpc.auth.getState()
  if (currentState.isAuthenticated) {
    await redirectIfAuthenticated()
    return
  }

  if (currentState.isAuthenticating) {
    applyAuthStateView(currentState)
    return
  }

  viewState.value = 'checking'
  try {
    await rpc.auth.checkAuthentication()
  }
  catch {
    viewState.value = 'idle'
    return
  }

  const nextState = await rpc.auth.getState()
  if (nextState.isAuthenticated) {
    await redirectIfAuthenticated()
    return
  }

  applyAuthStateView(nextState)
}

async function handleSignIn(): Promise<void> {
  // 防止重复触发登录流程，也避免与静默恢复并发
  if (viewState.value !== 'idle') {
    return
  }

  const authState = await rpc.auth.getState()
  if (authState.isAuthenticated) {
    await redirectIfAuthenticated()
    return
  }
  if (authState.isAuthenticating) {
    viewState.value = 'checking'
    return
  }

  viewState.value = 'submitting'
  try {
    await rpc.auth.login()
    await redirectIfAuthenticated()
  }
  catch {
    viewState.value = 'idle'
  }
  finally {
    // 如果 login 调用返回后仍未跳转（例如打开了 OAuth 窗口），状态会由事件通知维护
    // 如果没有打开窗口且未认证，则重置为 idle
    const finalState = await rpc.auth.getState()
    if (!finalState.isAuthenticating && !finalState.isAuthenticated) {
      viewState.value = 'idle'
    }
  }
}

onMounted(() => {
  void bootstrapLoginState()

  disposeAuthSessionReady = events.on('auth.sessionReady', () => {
    void redirectIfAuthenticated()
  })

  disposeAuthStateChanged = events.on('auth.stateChanged', (state) => {
    if (state.isAuthenticated) {
      void redirectIfAuthenticated()
      return
    }
    applyAuthStateView(state as any, { preserveSubmitting: true })
  })
})

onUnmounted(() => {
  if (disposeAuthSessionReady !== undefined) {
    disposeAuthSessionReady()
    disposeAuthSessionReady = undefined
  }
  if (disposeAuthStateChanged !== undefined) {
    disposeAuthStateChanged()
    disposeAuthStateChanged = undefined
  }
})
</script>

<template>
  <section class="login-page">
    <FocusScope
      :id="SPATIAL_NAV_SCOPE_IDS.login"
      :default-focus-id="SPATIAL_NAV_NODE_IDS.login.signIn"
    >
      <main class="login-content">
        <p class="login-content__desc">
          {{ loginStatusLine }}
        </p>

        <BrandedLoading
          v-if="isLoading"
          class="login-content__loading"
          size="sm"
          :label="loginActionLabel"
        />

        <Focusable
          v-else
          :id="SPATIAL_NAV_NODE_IDS.login.signIn"
          :scope-id="SPATIAL_NAV_SCOPE_IDS.login"
          :disabled="isLoginSubmitting"
          as="button"
          class="login-content__sign-in ui-action-button ui-action-button--brand"
          type="button"
          @click="() => void handleSignIn()"
        >
          {{ loginActionLabel }}
        </Focusable>
      </main>
    </FocusScope>
  </section>
</template>

<style scoped>
.login-page {
  position: relative;
  min-height: 100vh;
  color: var(--ui-page-text);
  background-color: var(--ui-page-bg);
}

.login-content {
  position: relative;
  z-index: 1;
  width: min(100%, var(--ui-login-content-width));
  margin: 0 auto;
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--ui-login-content-gap);
  padding:
    var(--ui-login-content-padding-top)
    var(--ui-page-inset)
    var(--ui-login-content-padding-bottom);
}

.login-content__desc {
  width: min(100%, var(--ui-login-desc-width));
  text-align: center;
  font-size: var(--ui-text-title-md);
  line-height: var(--ui-line-height-relaxed);
  font-weight: var(--ui-font-weight-medium);
  color: var(--ui-page-text-soft);
}

.login-content__loading {
  margin: 0 auto;
}

.login-content__sign-in {
  display: inline-flex;
  margin: 0 auto;
  width: min(100%, var(--ui-login-signin-width));
  cursor: pointer;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.login-content__sign-in[data-focused='true'] {
  transform: none;
  box-shadow: var(--shadow-xbox-focus);
}

.login-content__sign-in:disabled {
  opacity: 0.62;
  cursor: not-allowed;
  transform: none;
}

:global(html[data-ui-density='narrow']) .login-content__desc {
  font-size: 14px;
}
</style>
