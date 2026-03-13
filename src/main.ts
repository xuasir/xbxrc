import { createApp } from 'vue'
import App from './App.vue'
import { i18n, setUiLocale } from './i18n'
import { applyTheme } from './app/theme'

import { router } from './router'
import { rpc } from './services/rpc'
import './styles/base.css'
import './styles/tokens.css'
import './styles/theme.scss'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

async function bootstrap(): Promise<void> {
  try {
    // 首屏同步配置，避免 UI 状态与持久化配置不一致
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

  createApp(App).use(router).use(i18n).mount('#app')
}

void bootstrap()
