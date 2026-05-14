import { createApp } from 'vue'
import App from './App.vue'
import { applyTheme } from './app/theme'
import { i18n, setUiLocale } from './i18n'

import { ensureShellGamepadListening } from './navigation/core'
import { router } from './router'
import { rpc } from './services/rpc'
import { devWarn } from './shared/dev-log'
import {
  businessInputArbiter,
  toBusinessInputTracePayload,
} from './shared/gamepad/business-input-arbiter'
import './styles/base.css'
import './styles/tokens.css'
import './styles/theme.scss'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function syncBusinessInputScene(path: string): void {
  const streamScene = path.includes('/stream/')
  businessInputArbiter.patch({ appScene: streamScene ? 'stream' : 'shell' })
}

function installBusinessInputTraceBridge(): void {
  let lastSignature = ''
  businessInputArbiter.subscribe((snapshot) => {
    const payload = toBusinessInputTracePayload(snapshot)
    const signature = JSON.stringify(payload)
    if (signature === lastSignature) {
      return
    }
    lastSignature = signature
    void rpc.runtimeTrace.recordEvent({
      event: 'gamepadBusinessInputRouteChanged',
      payload,
    }).catch(() => {})
  })
}

async function bootstrap(): Promise<void> {
  ensureShellGamepadListening()
  businessInputArbiter.installGamepadGateBridge()
  installBusinessInputTraceBridge()
  syncBusinessInputScene(router.currentRoute.value.path)
  createApp(App).use(router).use(i18n).mount('#app')
  router.afterEach((to) => {
    syncBusinessInputScene(to.path)
  })
  void router.isReady().then(() => {
    syncBusinessInputScene(router.currentRoute.value.path)
  })

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
