import { createApp } from 'vue'
import App from './App.vue'
import { applyTheme } from './app/theme'
import { i18n, setUiLocale } from './i18n'

import { ensureShellGamepadListening } from './navigation/core'
import { router } from './router'
import { rpc } from './services/rpc'
import './styles/base.css'
import './styles/tokens.css'
import './styles/theme.scss'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

async function bootstrap(): Promise<void> {
  ensureShellGamepadListening()
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
    console.warn('[renderer] failed to sync config:', error)
  }
}

void bootstrap()
