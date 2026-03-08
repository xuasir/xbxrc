import { createI18n } from 'vue-i18n'
import en from './locales/en.json'
import zh from './locales/zh.json'

const UI_LOCALE_MESSAGES = {
  zh,
  en,
} as const

export type UiLocale = keyof typeof UI_LOCALE_MESSAGES

// 仅暴露当前渲染层真正支持的 UI 语言，其他 locale 统一回退到英文
export function resolveUiLocale(locale: unknown): UiLocale {
  if (typeof locale !== 'string') {
    return 'en'
  }

  const normalizedLocale = locale.trim().toLowerCase()
  if (normalizedLocale.startsWith('zh')) {
    return 'zh'
  }

  return 'en'
}

export const i18n = createI18n({
  legacy: false,
  locale: 'en',
  fallbackLocale: 'en',
  messages: UI_LOCALE_MESSAGES,
})

export function setUiLocale(locale: unknown): UiLocale {
  const resolvedLocale = resolveUiLocale(locale)
  i18n.global.locale.value = resolvedLocale
  return resolvedLocale
}
