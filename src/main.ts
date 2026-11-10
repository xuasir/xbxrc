import { createApp } from 'vue'
import App from './App.vue'
import { applyTheme } from './app/theme'
import { installUiMotionInput } from './app/ui-motion'
import { i18n, setUiLocale } from './i18n'

import { ensureShellGamepadListening } from './navigation/core'
import { router } from './router'
import { rpc } from './services/rpc'
import { devWarn } from './shared/dev-log'
import { installKeyboardFallbackBridge } from './shared/gamepad/keyboard-fallback-bridge'
import { initializeGamepadInput, inputController } from './shared/gamepad/runtime'
import './styles/base.css'
import './styles/tokens.css'
import './styles/theme.scss'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

async function bootstrap(): Promise<void> {
  installUiMotionInput()
  ensureShellGamepadListening()
  installKeyboardFallbackBridge()
  void initializeGamepadInput().catch(error => devWarn('[gamepad] initialization failed:', error))
  let lastSignature = ''
  let lastInputStatus: ReturnType<typeof inputController.getSnapshot> | undefined
  let lastSampling: ReturnType<typeof inputController.getSnapshot>['sampling']
  inputController.subscribe((snapshot) => {
    if (snapshot.sampling && snapshot.sampling !== lastSampling) {
      lastSampling = snapshot.sampling
      void rpc.runtimeTrace.recordEvent({ event: 'gamepadSamplingObserved', payload: snapshot.sampling }).catch(() => {})
    }
    if (lastInputStatus?.backend === snapshot.backend && lastInputStatus.owner === snapshot.owner
      && lastInputStatus.activeDeviceId === snapshot.activeDeviceId && lastInputStatus.isBackground === snapshot.isBackground
      && lastInputStatus.preferences.backgroundPolicy === snapshot.preferences.backgroundPolicy) {
      return
    }
    lastInputStatus = snapshot
    const payload = { backend: snapshot.backend, owner: snapshot.owner, activeDeviceId: snapshot.activeDeviceId, backgroundPolicy: snapshot.preferences.backgroundPolicy, isBackground: snapshot.isBackground }
    const signature = JSON.stringify(payload)
    if (signature === lastSignature)
      return
    lastSignature = signature
    void rpc.runtimeTrace.recordEvent({ event: 'gamepadInputChanged', payload }).catch(() => {})
  })
  createApp(App).use(router).use(i18n).mount('#app')

  try {
    // 不阻塞 Vue 挂载；启动优先保证首页与输入系统尽快可交互。
    const config = await rpc.config.get({ keys: ['locale', 'theme'] })
    if (isRecord(config)) {
      if (typeof config.locale === 'string') {
        setUiLocale(config.locale)
      }
      if (typeof config.theme === 'string') {
        applyTheme(config.theme as any)
      }
    }
  }
  catch (error) {
    devWarn('[renderer] failed to sync config:', error)
  }
}

void bootstrap()
