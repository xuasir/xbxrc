import { CONFIG_FIELD_DEFINITIONS } from '/src/shared/config/domain-definition.ts'
import { CONFIG_FIELD_RPC_GROUP } from '/src/pages/settings/setting-page-schema.ts'
import { navigationEngine } from '/src/navigation/core/engine.ts'
import { isFocusScopeAvailable, isFocusTarget } from '/src/navigation/core/focus-target.ts'
import { findNextFocusable } from '/src/navigation/core/pathfinding.ts'

// 只为此离线验证入口提供 RPC 数据；App、路由、输入后端与页面均由 src/main.ts 装配。
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
function navigationGraph() {
  const scope = [...document.querySelectorAll('[data-nav-zone]')].find(element =>
    navigationEngine.isTopmostScope(element.dataset.navZone) && isFocusScopeAvailable(element)) ?? document.body
  const elements = [...scope.querySelectorAll('[data-focusable="true"]')].filter(element => isFocusTarget(element, scope))
  const candidates = elements.map(element => {
    const rect = element.getBoundingClientRect()
    return { id: element.id, getBoundingClientRect: () => rect }
  })
  const edges = new Map(candidates.map(candidate => [candidate.id, ['up', 'down', 'left', 'right'].map(direction =>
    findNextFocusable(candidate, candidates.filter(target => target.id === candidate.id || !document.getElementById(target.id)?.contains(document.getElementById(candidate.id))), direction)?.id).filter(Boolean)]))
  const reached = new Set()
  const queued = [navigationEngine.getCurrentFocus()?.id ?? candidates[0]?.id]
  while (queued.length) {
    const id = queued.pop()
    if (!id || reached.has(id)) continue
    reached.add(id)
    queued.push(...edges.get(id) ?? [])
  }
  return { scope: scope.dataset.navZone ?? 'document', nodes: candidates.map(candidate => candidate.id), reachable: [...reached], unreachable: candidates.filter(candidate => !reached.has(candidate.id)).map(candidate => candidate.id) }
}
function record() {
  const current = {
    route: location.hash,
    viewport: { width: window.innerWidth, height: window.innerHeight },
    native: document.activeElement?.id ?? '',
    marked: [...document.querySelectorAll('[data-focused="true"]')].map(element => element.id),
    busy: [...document.querySelectorAll('[data-nav-busy="true"]')].map(element => element.id),
    pending: [...pending],
  }
  const signature = JSON.stringify(current)
  if (signature !== lastSnapshot) {
    timeline.push({ atMs: Math.round(performance.now()), ...current })
    lastSnapshot = signature
  }
  if (timeline.length > 400) timeline.shift()
  evidence.textContent = JSON.stringify({ instanceId, counters, keys, timeline, current, graph: navigationGraph() }, null, 2)
}
new MutationObserver(changes => {
  if (changes.some(change => change.target !== evidence)) record()
}).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-focused', 'data-nav-busy'] })
document.addEventListener('focusin', record)
window.addEventListener('resize', record)
document.addEventListener('keydown', event => {
  if (['j', 'k', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', '1', '2', '3', '4', 'Tab', 'Escape'].includes(event.key)) {
    keys.push({ key: event.key, atMs: Math.round(performance.now()) })
    for (const delay of [0, 250, 800, 1700]) setTimeout(record, delay)
  }
}, true)
function emit(event, payload) {
  for (const [id, listener] of listeners) {
    if (listener.event === event) callbacks.get(listener.handler)?.({ event, id, payload })
  }
}
const wait = ms => new Promise(resolve => setTimeout(resolve, ms))
const art = '/src/assets/console/series-x-transparent.png'
const video = new URL('./sample.mp4', import.meta.url).href
const hosts = [
  { id: 'fixture-console', serverId: 'fixture-console', name: '本地主机', consoleType: 'Series X', powerState: 'On', consoleStreamingEnabled: true, remoteManagementEnabled: true },
  { id: 'fixture-console-s', serverId: 'fixture-console-s', name: '备用主机', consoleType: 'Series S', powerState: 'ConnectedStandby', consoleStreamingEnabled: true, remoteManagementEnabled: true },
]
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
const image = i => `data:image/svg+xml;charset=utf-8,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="600" height="400"><defs><linearGradient id="a" x2="1" y2="1"><stop stop-color="${['#193f58','#44315f','#245844'][i%3]}"/><stop offset="1" stop-color="#071820"/></linearGradient></defs><rect width="600" height="400" fill="url(#a)"/><circle cx="400" cy="120" r="65" fill="#9fd9ad" opacity=".5"/><path d="M0 400 130 170 250 350 410 210 600 400" fill="#060e18"/><text x="30" y="65" fill="white" font-size="30" font-family="sans-serif">XBOX CLOUD</text></svg>`)}`
const cloudGames=Array.from({length:2000},(_,i)=>({
  id:`P${String(i).padStart(4,'0')}`, productId:`P${String(i).padStart(4,'0')}`, titleId:`stream-${i}`, xboxTitleId:i+1,
  name:i===0?'Halo Infinite':i===1?'最後一戰：無限':`云游戏 ${String(i).padStart(4,'0')}`, aliases:i===1?['Halo Infinite']:[],
  publisherName:'Xbox Game Studios', description:'探索与合作冒险', tileImageUrl:i===0?'/fixture-missing-cover.png':image(i), posterImageUrl:image(i),
  categories:i%2?['角色扮演']:['动作冒险'], supportedInputTypes:['Controller'], hasEntitlement:i!==2 && scenario!=='public', entitlementKnown:i!==3 && scenario!=='public',
  recentRank:i<4?3-i:null, newRank:i>=4&&i<8?7-i:null, popularRank:i>=8&&i<12?11-i:null, leavingRank:i>=12&&i<16?15-i:null,
  isRecentlyPlayed:i<4,isNew:i>=4&&i<8,
}))
let preferences={favorites:[{...cloudGames[7],productId:'MISSING',id:'MISSING',name:'已收藏 · 目录暂缺'}],pinned:['MISSING'],section:'',category:'',sort:'name'}
function catalog(cached) {
  return { scopeKey:'fixture-account-US', titles: scenario==='empty'?[]:cloudGames,
    refreshing:false, cacheState:cached?'stale':'fresh', updatedAt:Date.now()- (cached?600000:0),
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
  record()
  try {
    const mutation = namespace === 'config' && method === 'set'
      || namespace === 'data' && ['saveMedia', 'getHosts', 'getPersonalAccount', 'getConsoleLibrary', 'getGameHistory', 'getAchievements', 'getMedia', 'resolveMedia', 'refreshXcloudTitles'].includes(method)
      || namespace === 'updater' && ['check', 'setChannel'].includes(method)
    await wait(mutation ? (counters[name] === 1 && namespace === 'data' ? 250 : 1500) : 20)
    switch (name) {
      case 'auth.getState': return { provider: 'xal', isAuthenticating: false, isAuthenticated: authenticated, appLevel: 2 }
      case 'auth.logout': authenticated = false; emit('auth:state-changed', { provider: 'xal', isAuthenticated: false, isAuthenticating: false, appLevel: 0 }); return { loggedOut: true }
      case 'auth.checkAuthentication': return { provider: 'xal', startedSilentFlow: false }
      case 'config.getRuntimeFeatures': return { nativeMedia: false, nativeGamepad: false }
      case 'config.get': return Object.fromEntries(params.keys.map(key => [key, config[key]]))
      case 'config.getGroups': return groups()
      case 'config.set': Object.assign(config, params.patch); return { ...config }
      case 'app.getVersion': return '0.1.0'
      case 'app.getStartupFlags': return { fullscreen: false, autoConnect: '' }
      case 'app.isFullscreen': return false
      case 'data.getUserProfile': return { signedIn: true, gameDisplayName: '本地验证账户', gamertag: 'LocalValidation', gameDisplayPicRaw: art, gamerscore: '120', settings: {}, appLevel: 2 }
      case 'data.getHosts': case 'data.getRemoteConsoles': return hosts
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
      case 'data.getXcloudTitles': return catalog(true)
      case 'data.refreshXcloudTitles': return scenario === 'failure' ? { ...catalog(false), error: '目录刷新失败，保留缓存', cacheState: 'stale', sources: { newest: 'unavailable' } } : catalog(false)
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
          platforms: ['XboxSeriesX', 'XCloud'], screenshots: [image(1),image(2)],
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
  finally { pending.delete(name); record() }
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
