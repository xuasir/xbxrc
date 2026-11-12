import { createHostsFixture, readHomeEvidence } from './host-fixture.js'
import { CONFIG_FIELD_DEFINITIONS } from '/src/shared/config/domain-definition.ts'
import { CONFIG_FIELD_RPC_GROUP } from '/src/pages/settings/setting-page-schema.ts'
import { navigationEngine } from '/src/navigation/core/engine.ts'
import { cloudCatalog } from '/src/pages/xcloud/use-cloud-catalog.ts'
import { inputDispatcher, NavigationIntent } from '/src/navigation/core/input.ts'

// 只为此离线验证入口提供 RPC 数据；App、路由、输入后端与页面均由 src/main.ts 装配。
const size = Number(new URLSearchParams(location.search).get('size') ?? 2000)
const scenario = new URLSearchParams(location.search).get('scenario') ?? 'catalog'
const config = Object.fromEntries(Object.entries(CONFIG_FIELD_DEFINITIONS).map(([key, field]) => [key,
  field.control === 'toggle' ? false : field.options?.[0]?.value ?? field.input?.min ?? '',
]))
Object.assign(config, {
  locale: new URLSearchParams(location.search).get('language') ?? 'zh', theme: new URLSearchParams(location.search).get('theme') ?? 'dark', gamepad_control_source: 'gamepad', gamepad_background_policy: 'keep-active',
  gamepad_ui_navigation_enabled: true, gamepad_ui_haptics: false, gamepad_ui_audio: false,
  ui_audio: false, ui_haptics: false, debug: false, runtime_trace_mode: 'off',
  enable_audio_control: true, display_options: { sharpness: 0, saturation: 100, contrast: 100, brightness: 100 },
  xhome_bitrate_mode: 'Custom', xcloud_bitrate_mode: 'Custom', audio_bitrate_mode: 'Custom',
  xhome_bitrate: 30, xcloud_bitrate: 30, audio_bitrate: 128,
})
const callbacks = new Map()
const listeners = new Map()
let nextCallback = 0
let authenticated = scenario !== 'public'
let channel = 'stable'
const counters = {}
const pending = new Set()
const timeline = []
const keys = []
const instanceId = crypto.randomUUID()
const evidence = document.getElementById('project-validation-results')
let lastSnapshot = ''
const frameGaps = []
const longTasks = []
const navigationSamples = []
let navigationStart
navigationEngine.onFocusChange(element => {
  if (navigationStart && element && element.id !== navigationStart.source) {
    navigationSamples.push({ intent: navigationStart.intent, elapsedMs: performance.now() - navigationStart.at, target: element.id })
    navigationStart = undefined
  }
})
let previousFrame = performance.now()
function frame(now) {
  frameGaps.push(now - previousFrame)
  previousFrame = now
  if (frameGaps.length > 600) frameGaps.shift()
  requestAnimationFrame(frame)
}
requestAnimationFrame(frame)
try {
  new PerformanceObserver(list => longTasks.push(...list.getEntries().map(entry => ({ start: entry.startTime, duration: entry.duration })))).observe({ type: 'longtask', buffered: true })
} catch {}
const percentile = (samples, percent) => [...samples].sort((a,b) => a-b)[Math.floor(samples.length * percent)] ?? 0
function record() {
  const current = {
    route: location.hash,
    home: readHomeEvidence(),
    hostFixture: hostFixture.state(),
    viewport: { width: window.innerWidth, height: window.innerHeight },
    native: document.activeElement?.id ?? '',
    marked: [...document.querySelectorAll('[data-focused="true"]')].map(element => element.id),
    busy: [...document.querySelectorAll('[data-nav-busy="true"]')].map(element => element.id),
    pending: [...pending],
    cards: document.querySelectorAll('[data-nav-page-active="true"] .game-card').length,
    candidates: document.querySelectorAll('[data-nav-page-active="true"] [data-focusable="true"]').length,
    images: document.querySelectorAll('[data-nav-page-active="true"] img').length,
    windows: [...document.querySelectorAll('[data-nav-page-active="true"] [data-cloud-window]')].map(element => ({ section: element.dataset.cloudWindow, mounted: element.querySelectorAll('.game-card').length })),
    catalogMetrics: { ...cloudCatalog.metrics },
    catalogSubscribers: cloudCatalog.subscriberCount,
    navigationP95Ms: percentile(navigationSamples.map(sample => sample.elapsedMs), .95),
    frameP95Ms: percentile(frameGaps, .95),
    longTasks: longTasks.slice(-8),
  }
  const signature = JSON.stringify(current)
  if (signature !== lastSnapshot) {
    timeline.push({ atMs: Math.round(performance.now()), ...current })
    lastSnapshot = signature
  }
  if (timeline.length > 100) timeline.shift()
  evidence.textContent = JSON.stringify({ instanceId, counters, keys, timeline, current, size, navigationSamples, lifecycleListeners: [...listeners.values()].map(listener => listener.event) }, null, 2)
}
let pendingRecord = false
function scheduleRecord() {
  if (pendingRecord) return
  pendingRecord = true
  requestAnimationFrame(() => { pendingRecord = false; record() })
}
new MutationObserver(changes => {
  if (changes.some(change => change.target !== evidence && !evidence.contains(change.target))) scheduleRecord()
}).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-focused', 'data-nav-busy'] })
document.addEventListener('focusin', scheduleRecord)
window.addEventListener('resize', scheduleRecord)
document.addEventListener('keydown', event => {
  keys.push({ key: event.key, atMs: Math.round(performance.now()) })
  scheduleRecord()
}, true)
function emit(event, payload) {
  for (const [id, listener] of listeners) {
    if (listener.event === event) callbacks.get(listener.handler)?.({ event, id, payload })
  }
}
const wait = ms => new Promise(resolve => setTimeout(resolve, ms))
const art = new URL('../../../src/assets/console/series-x-transparent.png',import.meta.url).href
const video = ''
const hosts = [
  { id: 'fixture-console', serverId: 'fixture-console', name: '本地主机', consoleType: 'Series X', powerState: 'On', consoleStreamingEnabled: true, remoteManagementEnabled: true },
  { id: 'fixture-console-s', serverId: 'fixture-console-s', name: '备用主机', consoleType: 'Series S', powerState: 'ConnectedStandby', consoleStreamingEnabled: true, remoteManagementEnabled: true },
]
const hostFixture = createHostsFixture({ emit, record, counters })
const games = Array.from({ length: 12 }, (_, index) => ({
  xboxTitleId: `game-${index + 1}`, name: `本地游戏 ${index + 1}`, description: '离线前端交互验证数据',
  scid: null, productId: `product-${index + 1}`, artworkUrl: art, lastPlayedAt: `2026-11-${String(9 - index % 8).padStart(2, '0')}T01:00:00Z`,
  progress: { unlockedCount: index % 3 ? 2 : 3, totalCount: 3, earnedGamerscore: index % 3 ? 20 : 30, totalGamerscore: 30 },
}))
const achievements = ['本地成就一', '本地成就二', '本地隐藏成就'].map((name, index) => ({
  id: `achievement-${index + 1}`, scid: null, name, description: '已解锁的成就说明', lockedDescription: '达成目标后解锁',
  imageUrl: art, isSecret: index === 2, isUnlocked: index === 0, gamerscore: 10, progressPercentage: index === 0 ? 100 : 0,
  requirements: [], unlockedAt: index === 0 ? '2026-11-09T00:00:00Z' : null,
}))
const person = (index, online = true) => ({
  xuid: `fixture-friend-${index}`, displayName: `本地好友 ${index}`, gamertag: `LocalPlayer${index}`,
  avatarUrl: art, gamerscore: '120', presenceState: online ? 'Online' : 'Offline', presenceText: online ? '正在游玩本地游戏' : null,
  currentTitleId: online ? 'game-1' : null, isFollowedByCaller: true, isFollowingCaller: true,
})
const media = kind => Array.from({ length: 3 }, (_, index) => ({
  id: `${kind}-${index + 1}`, kind, xboxTitleId: `game-${index % 2 + 1}`, titleName: `本地游戏 ${index % 2 + 1}`,
  capturedAt: `2026-11-${String(9 - index).padStart(2, '0')}T00:00:00Z`, durationSeconds: kind === 'clip' ? 24 : null,
  thumbnailUrl: art, url: kind === 'clip' ? video : art, expiresAt: null,
}))
const query = (data, params = {}) => ({ accountId: 'fixture-account', targetId: params.targetXuid ?? 'fixture-account', updatedAt: Date.now(), data })
const page = (items, params = {}) => query({ items: params.continuation ? items.slice(2) : items.slice(0, 2), continuation: params.continuation ? null : 'next' }, params)
function groups() {
  const result = { app: {}, streaming: {}, host: {}, xcloud: {}, input: {} }
  for (const [key, value] of Object.entries(config)) result[CONFIG_FIELD_RPC_GROUP[key] ?? 'input'][key] = value
  return result
}
const artwork = new URL('./cover.svg?no-inline', import.meta.url).href
const image = i => `${artwork}?item=${i}`
let cloudGames=Array.from({length:size},(_,i)=>({
  id:`P${String(i).padStart(4,'0')}`, productId:`P${String(i).padStart(4,'0')}`, titleId:`stream-${i}`, xboxTitleId:i+1,
  name:i===0?'Halo Infinite':i===1?'最後一戰：無限':`云游戏 ${String(i).padStart(4,'0')}`, aliases:i===1?['Halo Infinite']:[],
  publisherName:'Xbox Game Studios', description:('探索、协作与冒险世界。完整描述供多词和分类搜索。 '.repeat(24))+i, tileImageUrl:i===0?'/fixture-missing-cover.png':image(i), posterImageUrl:image(i),
  categories:i%2?['角色扮演']:['动作冒险'], supportedInputTypes:['Controller'], hasEntitlement:i!==2 && scenario!=='public', entitlementKnown:i!==3 && scenario!=='public',
  recentRank:i<25?24-i:null, newRank:i>=25&&i<55?54-i:null, popularRank:i>=55&&i<85?84-i:null, leavingRank:i>=85&&i<99?98-i:null,
  isRecentlyPlayed:i<25,isNew:i>=25&&i<55,
}))
let preferences={favorites:[{...cloudGames[7],productId:'MISSING',id:'MISSING',name:'已收藏 · 目录暂缺'}, ...(scenario === 'empty' ? [] : cloudGames.filter((_,index) => index%3 === 0).slice(0,600))],pinned:['MISSING'],section:'',category:'',sort:'name'}
let contentRevision = 'r0'
let updatedAt = Date.now()
let refreshing = false
function catalog(cached) {
  return { scopeKey:'fixture-account-US', contentRevision, freshUntil: updatedAt + 600000, titles: scenario==='empty'?[]:cloudGames,
    refreshing, cacheState:cached?'stale':'fresh', updatedAt,
    access:scenario==='public'?'browse':'account', sources:{ catalog:'fresh',products:'fresh',recent:'fresh',newest:'fresh',popular:'fresh',leaving:'fresh' }, error:null }
}
async function rpc(namespace, method, params = {}) {
  const name = `${namespace}.${method}`
  if (namespace === 'runtimeTrace') {
    if (method === 'getPolicy') return { enabled: false, mode: 'off', maxEventBytes: 16384, producerBatchSize: 64 }
    return { accepted: true }
  }
  counters[name] = (counters[name] ?? 0) + 1
  pending.add(name)
  scheduleRecord()
  try {
    const mutation = namespace === 'config' && method === 'set'
      || namespace === 'data' && ['saveMedia', 'getHosts', 'getPersonalAccount', 'getConsoleLibrary', 'getGameHistory', 'getAchievements', 'getMedia', 'resolveMedia', 'refreshXcloudTitles'].includes(method)
      || namespace === 'updater' && ['check', 'setChannel'].includes(method)
    await wait(mutation ? (counters[name] === 1 && namespace === 'data' ? 250 : 1500) : 20)
    switch (name) {
      case 'auth.getState': return { provider: 'xal', isAuthenticating: false, isAuthenticated: authenticated, appLevel: authenticated ? 2 : 0 }
      case 'auth.logout': authenticated = false; emit('xbxrc:auth:state-changed', { provider: 'xal', isAuthenticated: false, isAuthenticating: false, appLevel: 0 }); return { loggedOut: true }
      case 'auth.checkAuthentication': return { provider: 'xal', startedSilentFlow: false }
      case 'config.getRuntimeFeatures': return { nativeMedia: false, nativeGamepad: false }
      case 'config.get': return Object.fromEntries(params.keys.map(key => [key, config[key]]))
      case 'config.getGroups': return groups()
      case 'config.set': Object.assign(config, params.patch); return { ...config }
      case 'app.getVersion': return '0.1.0'
      case 'app.getStartupFlags': return { fullscreen: false, autoConnect: '' }
      case 'app.isFullscreen': return false
      case 'data.getUserProfile': return { signedIn: true, gameDisplayName: '本地验证账户', gamertag: 'LocalValidation', gameDisplayPicRaw: art, gamerscore: '120', settings: {}, appLevel: 2 }
      case 'data.getRemoteConsoles': return hosts
      case 'data.getHosts': case 'data.getHostsSnapshot': case 'data.refreshHosts': case 'data.powerOnConsole': case 'data.powerOffConsole': return await hostFixture.invoke(method, params)
      case 'data.getPersonalAccount': return query({ profile: person(params.targetXuid ? 1 : 0), friends: [person(1), person(2, false), person(3)], notices: [] }, params)
      case 'data.getGameHistory': return page(games, params)
      case 'data.getLibraryGame': return query(games.find(game => game.xboxTitleId === params.xboxTitleId) ?? games[0], params)
      case 'data.getAchievements': return page(achievements, params)
      case 'data.getPlaytimes': return query(params.xboxTitleIds.map(xboxTitleId => ({ xboxTitleId, minutes: 120, error: null })), params)
      case 'data.getConsoleLibrary': return query({ consoleId: params.consoleId, powerState: 'On', notices: [], storage: [
        { id: 'internal', name: '内部存储', freeBytes: 200e9, totalBytes: 1e12 }, { id: 'external', name: '扩展存储', freeBytes: 400e9, totalBytes: 2e12 },
      ], games: games.map((game, index) => ({ ...game, id: `installed-${index + 1}`, storageDeviceId: index % 2 ? 'external' : 'internal', sizeBytes: (index + 1) * 1e9, installedAt: game.lastPlayedAt })) }, params)
      case 'data.getMedia': return page(media(params.kind), params)
      case 'data.resolveMedia': return query(media(params.kind).find(item => item.id === params.id), params)
      case 'data.saveMedia': return { saved: true, canceled: false }
      case 'data.getXcloudTitles': return catalog(false)
      case 'data.getCloudCatalogStatus': { const { titles, ...status } = catalog(false); return status }
      case 'data.getCloudTitleSummary': return { scopeKey: 'fixture-account-US', title: scenario === 'empty' ? null : cloudGames.find(title => title.productId === params.productId) ?? null }
      case 'data.prioritizeCloudTitles': return true
      case 'data.refreshXcloudTitles': {
        if (scenario === 'failure') return { ...catalog(false), error: '目录刷新失败，保留缓存', cacheState: 'stale', sources: { newest: 'unavailable' } }
        refreshing = true
        if (scenario !== 'empty') {
          for (let batch = 0; batch < 4; batch++) {
            await wait(300)
            const baseRevision = contentRevision
            contentRevision = `r${(Number(contentRevision.slice(1)) || 0)+1}`
            updatedAt = Date.now()
            const upserts = cloudGames.slice(batch * 75, batch * 75 + 75).map(title => ({ ...title, description: `${title.description} 更新资料` }))
            const changed = new Map(upserts.map(title => [title.productId, title]))
            cloudGames = cloudGames.map(title => changed.get(title.productId) ?? title)
            emit('xbxrc:data:xcloud-catalog-updated', { ...catalog(false), titles: [], reason: 'refresh', patch: { baseRevision, upserts, removedProductIds: [] } })
          }
        }
        refreshing = false
        emit('xbxrc:data:xcloud-catalog-updated', { ...catalog(false), titles: [], reason: 'completed', patch: { baseRevision: contentRevision, upserts: [], removedProductIds: [] } })
        return catalog(false)
      }
      case 'data.getCloudPreferences': return ['fixture-account', preferences]
      case 'data.setCloudPreferences': preferences = params.preferences; return preferences
      case 'data.resolveCloudTitle': return cloudGames.find(t => t.productId === params.productId)
      case 'data.getCloudWait': await wait(80); return { seconds: 35, updatedAt: Date.now() }
      case 'data.cancelCloudWait': return true
      case 'data.getCloudDetail': {
        if (scenario === 'detail-failure') throw new Error('游戏资料暂不可用，请重试')
        const title=cloudGames.find(t=>t.productId===params.productId)
        return { stale: scenario === 'failure', updatedAt: Date.now(), detail: {
          product: { ...title, heroImageUrl: image(0), description: '进入广阔的游戏世界，探索、建设与好友协作。\n支持跨设备云存档，随时继续上一段旅程。' },
          developer: 'Xbox Game Studios', releaseDate: '2026-10-20T00:00:00Z', rating: '12+', attributes: ['单人游戏', '在线多人', 'Xbox 云存档'],
          platforms: ['XboxSeriesX', 'XCloud'], screenshots: Array.from({length:10},(_,index)=>image(10000+index)),
          languages: [{ locale: 'zh-CN', interface:1, audio:2, subtitles:1 }, { locale:'en-US',interface:1,audio:1,subtitles:1 }, {locale:'ja-JP',interface:null,audio:7,subtitles:null}], accessibility:['可调字幕', '重新映射按键'],
        } }
      }
      case 'data.primeXcloudTitles': return { primed: true }
      case 'updater.getChannel': return { channel }
      case 'updater.setChannel': channel = params.channel; return { channel }
      case 'updater.check': return { currentVersion: '0.1.0', updateAvailable: false, version: null, notes: null, date: null }
      case 'streaming.listActiveSessions': return { sessions: [] }
      case 'streaming.getSessionProgress': return null
      case 'streaming.cancelStartup': return { cancelled: true }
      case 'streaming.startSession': throw new Error('本地前端验证：串流连接需要实机服务；可检查返回和故障弹层')
      default: throw new Error(`离线验证尚未提供 ${name}`)
    }
  }
  finally { pending.delete(name); scheduleRecord() }
}
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event, id) => listeners.delete(id) }
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
  convertFileSrc: path => path,
  transformCallback(callback) { const id = ++nextCallback; callbacks.set(id, callback); return id },
  unregisterCallback: id => callbacks.delete(id),
  async invoke(command, args = {}) {
    if (command === 'rpc_invoke') {
      try { return { ok: true, data: await rpc(args.payload.namespace, args.payload.method, args.payload.params) } }
      catch (error) { return { ok: false, error: { code: 'OFFLINE_VALIDATION', message: error.message } } }
    }
    if (command === 'plugin:event|listen') { const id = ++nextCallback; listeners.set(id, args); return id }
    if (command === 'plugin:event|unlisten') { listeners.delete(args.eventId); return }
    if (command === 'plugin:window|is_focused') return true
    throw new Error(`离线验证尚未提供 ${command}`)
  },
}
record()

window.__cloudValidation = {
  sample() { record(); return JSON.parse(evidence.textContent) },
  async scrollToIndex(index) {
    const root = document.querySelector('[data-cloud-window="xcloud-all"]')
    const scroller = document.querySelector('.xcloud-page__content')
    const columns = Number(root.dataset.cloudColumns)
    const gap = Number.parseFloat(getComputedStyle(root).columnGap)
    const cellSize = (root.clientWidth - (columns-1)*gap)/columns
    const offset = root.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop
    scroller.scrollTop = offset + Math.floor(index/columns) * (cellSize+gap)
    await wait(80)
    record()
  },
  dispatch: intent => inputDispatcher.dispatch(NavigationIntent[intent]),
  focus: id => navigationEngine.focusElement(document.getElementById(id), false, true),
  games: () => cloudGames,
}

// 可见验证控件通过现有意图入口回放；浏览器工具只需点击按钮和读取 DOM 证据。
const controls = document.createElement('details')
controls.id = 'cloud-validation-controls'
controls.style.cssText = 'position:fixed;right:12px;bottom:8px;z-index:10000;background:#20282a;color:white;padding:6px;border:1px solid #61726c;border-radius:6px;font:12px sans-serif;max-width:280px'
controls.innerHTML = `<summary>验证操作</summary><div style="display:grid;gap:6px;margin-top:8px">
  <label>目录索引 <input id="cloud-validation-index" type="number" value="2000" style="width:85px"></label>
  <button id="cloud-validation-jump" type="button">滚至索引并聚焦</button>
  <label>焦点节点 <input id="cloud-validation-focus" style="width:160px"></label>
  <button id="cloud-validation-apply-focus" type="button">聚焦节点</button>
  <div>${[['Up','上'],['Down','下'],['Left','左'],['Right','右'],['Action','A'],['Back','B'],['TabPrev','LT'],['TabNext','RT'],['PagePrev','LB'],['PageNext','RB']].map(([intent,label])=>`<button type="button" data-validation-intent="${intent}">${label}</button>`).join(' ')}</div>
  <button type="button" data-validation-intent="Down" data-repeat="40">下移 ×40</button>
  <button type="button" data-validation-intent="Right" data-repeat="30">右移 ×30</button>
  <button id="cloud-validation-record" type="button">记录当前状态</button>
  <output id="cloud-validation-output"></output>
</div>`
document.body.append(controls)
const controlOutput = controls.querySelector('output')
controls.addEventListener('pointerdown', event => {
  if (event.target instanceof HTMLButtonElement) event.preventDefault()
})
controls.addEventListener('click', async event => {
  const button = event.target.closest('button')
  if (!button) return
  controlOutput.dataset.running = 'true'
  try {
    if (button.id === 'cloud-validation-jump') {
      await window.__cloudValidation.scrollToIndex(Number(document.getElementById('cloud-validation-index').value))
      const root = document.querySelector('[data-cloud-window="xcloud-all"]')
      const viewport = document.querySelector('.xcloud-page__content').getBoundingClientRect()
      const card = [...root.querySelectorAll('.game-card')].find(element => {
        const rect = element.getBoundingClientRect()
        return rect.bottom > viewport.top + 1 && rect.top < viewport.bottom - 1
      })
      navigationEngine.focusElement(card?.querySelector('.game-card__main'), false, true)
    }
    else if (button.id === 'cloud-validation-apply-focus') {
      navigationEngine.focusElement(document.getElementById(document.getElementById('cloud-validation-focus').value), false, true)
    }
    else if (button.dataset.validationIntent) {
      for (let repeat = 0; repeat < Number(button.dataset.repeat ?? 1); repeat++) {
        const intent = button.dataset.validationIntent
        navigationStart = ['Up','Down','Left','Right'].includes(intent) ? { intent, at: performance.now(), source: navigationEngine.getCurrentFocus()?.id } : undefined
        inputDispatcher.dispatch(NavigationIntent[button.dataset.validationIntent])
        await wait(25)
      }
    }
    controlOutput.textContent = navigationEngine.getCurrentFocus()?.id ?? '无焦点'
    record()
  }
  catch (error) { controlOutput.textContent = error.message }
  finally { controlOutput.dataset.running = 'false'; record() }
})
