import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'
import { computed, onScopeDispose, ref } from 'vue'
import { loginRecoveryAction } from '../../pages/login-recovery'
import { diagnosticIssueId } from '../../services/diagnostics'
import { onRpcError, rpc } from '../../services/rpc'
import { devWarn } from '../../shared/dev-log'

export function useAuthRecovery(options: {
  route: Pick<RouteLocationNormalizedLoaded, 'fullPath'>
  router: Pick<Router, 'push'>
  t: (key: string) => string
}) {
  const open = ref(false)
  const busy = ref(false)
  const issueId = ref<string>()
  const failureMessage = ref('')
  const message = computed(() => failureMessage.value || options.t('login.reauthenticateHelp'))

  // Rust 已完成自动续期和一次恢复重试；此处只承接最终鉴权失效。
  onScopeDispose(onRpcError((request, error) => {
    if (request.namespace === 'data' && loginRecoveryAction(error) === 'login' && !busy.value) {
      issueId.value = diagnosticIssueId(error)
      failureMessage.value = ''
      open.value = true
    }
  }))

  function cancel(): void {
    if (busy.value)
      return
    open.value = false
    failureMessage.value = ''
  }

  async function confirm(): Promise<void> {
    if (!open.value || busy.value)
      return
    const redirect = options.route.fullPath
    busy.value = true
    failureMessage.value = ''
    try {
      const result = await rpc.auth.logout()
      if (!result.loggedOut)
        throw new Error('logoutRejected')
      const failure = await options.router.push({ name: 'login', query: { redirect } })
      if (failure)
        throw new Error('loginNavigationFailed')
      open.value = false
    }
    catch (error) {
      issueId.value = diagnosticIssueId(error)
      devWarn('[Auth] open reauthentication failed:', error)
      failureMessage.value = options.t('login.reauthenticateFailed')
    }
    finally {
      busy.value = false
    }
  }

  return { open, busy, message, issueId, cancel, confirm }
}
