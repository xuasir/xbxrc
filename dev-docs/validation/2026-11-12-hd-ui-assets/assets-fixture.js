import '../2026-11-11-gamepad-locked-input-and-extra-buttons/project-fixture.js'

const params = new URLSearchParams(location.search)
const preferences = { theme: params.get('theme') ?? 'dark', locale: params.get('language') ?? 'zh' }
const hosts = [
  { id: 'fixture-console', serverId: 'fixture-console', name: '客厅 Xbox', consoleType: 'Series X', powerState: 'On', consoleStreamingEnabled: true, remoteManagementEnabled: true },
  { id: 'fixture-console-s', serverId: 'fixture-console-s', name: '书房 Xbox', consoleType: 'Series S', powerState: 'ConnectedStandby', consoleStreamingEnabled: true, remoteManagementEnabled: true },
]
const originalInvoke = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__)
// 独立入口补齐当前首页缓存合同；页面、焦点路由及原验证夹具继续共用。
window.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
  const payload = args.payload
  if (command === 'rpc_invoke' && payload.namespace === 'data' && ['getHostsSnapshot', 'refreshHosts'].includes(payload.method)) {
    return { ok: true, data: { scopeKey: 'fixture-account', hosts, updatedAt: Date.now(), freshUntil: Date.now() + 60_000 } }
  }
  const result = await originalInvoke(command, args)
  if (command === 'rpc_invoke' && result?.ok && payload.namespace === 'config') {
    if (payload.method === 'set') {
      for (const key of Object.keys(preferences)) {
        if (key in payload.params.patch) preferences[key] = payload.params.patch[key]
      }
    }
    const groups = payload.method === 'getGroups' ? Object.values(result.data) : [result.data]
    for (const group of groups) {
      for (const [key, value] of Object.entries(preferences)) {
        if (group && key in group) group[key] = value
      }
    }
  }
  return result
}

const errors = []
const recordError = message => {
  errors.push(String(message))
  document.getElementById('asset-errors').textContent = JSON.stringify(errors)
}
window.addEventListener('error', event => recordError(event.message))
window.addEventListener('unhandledrejection', event => recordError(event.reason?.message ?? event.reason))

const tools = document.createElement('nav')
tools.id = 'asset-validation-tools'
tools.setAttribute('aria-label', '素材验证操作')
tools.style.cssText = 'position:fixed;right:12px;bottom:48px;z-index:10001;display:flex;flex-wrap:wrap;gap:6px;max-width:340px;padding:8px;background:#18221b;color:white;border:1px solid #657966;border-radius:8px;font:12px sans-serif'
const query = values => `?${new URLSearchParams({ ...preferences, language: preferences.locale, ...values })}#/xhome`
tools.innerHTML = `<a href="${query({ theme: 'dark' })}">深色</a><a href="${query({ theme: 'light' })}">浅色</a><a href="${query({ language: 'zh' })}">中文</a><a href="${query({ language: 'en' })}">English</a><a href="./icons.html?theme=${preferences.theme}">图标预览</a><button id="asset-hide-tools" type="button">隐藏验证操作</button>`
for (const link of tools.querySelectorAll('a')) link.style.cssText = 'color:white;padding:4px;text-decoration:underline'
tools.querySelector('button').addEventListener('click', () => { tools.hidden = true; tools.style.display = 'none' })
document.body.append(tools)

await import('../2026-11-11-gamepad-locked-input-and-extra-buttons/gamepad-fixture.js')
if (!location.hash) location.hash = '/xhome'
await import('/src/main.ts')
