import { createApp } from 'vue'
import { createI18n } from 'vue-i18n'
import zh from '@/i18n/locales/zh.json'
import en from '@/i18n/locales/en.json'
import { navigationEngine } from '@/navigation/core/engine'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import Preview from './Preview.vue'
import '@/styles/base.css'
import '@/styles/tokens.css'
import '@/styles/theme.scss'
const params = new URLSearchParams(location.search)
document.documentElement.dataset.theme = params.get('theme') ?? 'dark'
navigationEngine.start()
window.addEventListener('keydown', event => {
  const intents = { ArrowUp: NavigationIntent.Up, ArrowDown: NavigationIntent.Down, ArrowLeft: NavigationIntent.Left, ArrowRight: NavigationIntent.Right, Enter: NavigationIntent.Action }
  const intent = intents[event.key]
  if (intent) { event.preventDefault(); inputDispatcher.dispatch(intent, event, 'keyboard') }
})
createApp(Preview).use(createI18n({ legacy: false, locale: params.get('locale') ?? 'zh', messages: { zh, en } })).mount('#app')
