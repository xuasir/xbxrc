export type ThemeMode = 'dark' | 'light'

export function applyTheme(theme: ThemeMode): void {
  document.documentElement.dataset.theme = theme
  document.documentElement.style.colorScheme = theme
}

/**
 * 监听主题变化并在初始化时应用。
 * 由于设置页面通过 rpc.config.set 修改配置后会触发 syncConfigGroups，
 * 我们可以在 App.vue 中统一监听配置变化或在初始化时设置。
 */
export function setupTheme(initialTheme: ThemeMode = 'dark'): void {
  applyTheme(initialTheme)
}
