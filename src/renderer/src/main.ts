import './styles/base.css'
import './styles/theme.scss'

import { createApp } from 'vue'
import App from './App.vue'
import { router } from './router'
import { i18n, setUiLocale } from './i18n'
import { rpc } from './services/rpc'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

async function bootstrap(): Promise<void> {
  try {
    // 首屏先和后端 locale 对齐，避免 UI 文案固定在默认语言
    const config = await rpc.config.get({ keys: ['locale'] })
    if (isRecord(config)) {
      setUiLocale(config.locale)
    }
  } catch (error) {
    console.warn('[renderer] failed to sync ui locale from config:', error)
  }

  createApp(App).use(router).use(i18n).mount('#app')
}

void bootstrap()
