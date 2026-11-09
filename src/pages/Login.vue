<script setup lang="ts">
import type { AuthTraceContext } from '../services/auth-trace'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { createAuthTraceOperationId, recordAuthTrace } from '../services/auth-trace'
import { events } from '../services/events'
import { rpc } from '../services/rpc'
import { loginRecoveryAction } from './login-recovery'

const { t } = useI18n()
const router = useRouter()
const route = useRoute()

type LoginViewState = 'checking' | 'idle' | 'submitting'

const viewState = ref<LoginViewState>('checking')
const recoveryAction = ref<'retry' | 'login' | null>(null)
const loginTraceId = createAuthTraceOperationId()

function trace(event: string, context: AuthTraceContext = {}): void {
  recordAuthTrace(event, { operationId: loginTraceId, source: 'login-page', viewState: viewState.value, ...context })
}

watch(viewState, () => trace('loginViewStateChanged'), { flush: 'sync' })

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
  if (recoveryAction.value) {
    return t(recoveryAction.value === 'retry' ? 'login.restoreRetryHelp' : 'login.restoreLoginHelp')
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
  if (recoveryAction.value) {
    return t(recoveryAction.value === 'retry' ? 'login.retryRestore' : 'streamPage.failure.login')
  }
  return t('login.signInSimple')
})

async function redirectIfAuthenticated(trigger: AuthTraceContext['trigger'] = 'bootstrap'): Promise<boolean> {
  const startedAt = performance.now()
  const operationId = createAuthTraceOperationId()
  const context = { operationId, parentOperationId: loginTraceId, trigger }
  const to = router.resolve(redirectTarget.value)
  trace('loginRedirectStarted', { ...context, to })
  try {
    const authState = await rpc.auth.getState()
    trace('loginAuthStateObserved', { ...context, state: authState })
    if (authState.isAuthenticated) {
      const failure = await router.replace(redirectTarget.value)
      trace('loginRedirectCompleted', { ...context, to: router.currentRoute.value, durationMs: Math.round(performance.now() - startedAt), outcome: failure ? 'failed' : 'success', navigationFailureType: failure?.type })
      return true
    }
    trace('loginRedirectCompleted', { ...context, to, durationMs: Math.round(performance.now() - startedAt), outcome: 'skipped' })
    return false
  }
  catch (error) {
    trace('loginRedirectFailed', { ...context, to, durationMs: Math.round(performance.now() - startedAt), error })
    throw error
  }
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
  recoveryAction.value = null
  viewState.value = 'checking'
  const startedAt = performance.now()
  trace('loginRestoreStarted', { trigger: 'bootstrap' })
  try {
    const currentState = await rpc.auth.getState()
    trace('loginAuthStateObserved', { trigger: 'bootstrap', state: currentState })
    if (currentState.isAuthenticated) {
      trace('loginRestoreCompleted', { state: currentState, outcome: 'success', durationMs: Math.round(performance.now() - startedAt) })
      await redirectIfAuthenticated()
      return
    }

    if (currentState.isAuthenticating) {
      applyAuthStateView(currentState)
      trace('loginRestoreWaiting', { state: currentState, outcome: 'waiting', durationMs: Math.round(performance.now() - startedAt) })
      return
    }

    viewState.value = 'checking'
    trace('loginAuthCheckStarted')
    try {
      await rpc.auth.checkAuthentication()
      trace('loginAuthCheckCompleted', { outcome: 'success' })
    }
    catch (error) {
      recoveryAction.value = loginRecoveryAction(error)
      viewState.value = 'idle'
      trace('loginRestoreCompleted', { outcome: 'failed', error, durationMs: Math.round(performance.now() - startedAt) })
      return
    }

    const nextState = await rpc.auth.getState()
    trace('loginAuthStateObserved', { trigger: 'after-check', state: nextState })
    trace('loginRestoreCompleted', { state: nextState, outcome: nextState.isAuthenticating ? 'waiting' : 'success', durationMs: Math.round(performance.now() - startedAt) })
    if (nextState.isAuthenticated) {
      await redirectIfAuthenticated('after-check')
      return
    }

    applyAuthStateView(nextState)
  }
  catch (error) {
    recoveryAction.value = loginRecoveryAction(error)
    viewState.value = 'idle'
    trace('loginRestoreFailed', { error, durationMs: Math.round(performance.now() - startedAt) })
  }
}

async function handleLoginAction(): Promise<void> {
  if (viewState.value !== 'idle')
    return
  if (recoveryAction.value === 'retry') {
    await bootstrapLoginState()
    return
  }
  await handleSignIn()
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
  recoveryAction.value = null
  const startedAt = performance.now()
  trace('loginSignInStarted', { trigger: 'sign-in' })
  try {
    await rpc.auth.login()
    trace('loginSignInCompleted', { outcome: 'success', durationMs: Math.round(performance.now() - startedAt) })
    await redirectIfAuthenticated('sign-in')
  }
  catch (error) {
    trace('loginSignInFailed', { error, durationMs: Math.round(performance.now() - startedAt) })
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
  trace('loginMounted')
  void bootstrapLoginState()

  disposeAuthSessionReady = events.on('auth.sessionReady', () => {
    trace('loginAuthEventReceived', { trigger: 'session-ready' })
    void redirectIfAuthenticated('session-ready')
  })

  disposeAuthStateChanged = events.on('auth.stateChanged', (state) => {
    trace('loginAuthEventReceived', { trigger: 'state-changed', state })
    if (state.isAuthenticated) {
      void redirectIfAuthenticated('state-changed')
      return
    }
    applyAuthStateView(state as any, { preserveSubmitting: true })
  })
})

onUnmounted(() => {
  trace('loginUnmounted')
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
          @click="() => void handleLoginAction()"
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
