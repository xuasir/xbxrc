import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { runInNewContext } from 'node:vm'

// 执行真实详情 setup 与 KeepAlive hooks，宿主仅记录路由触发的请求。
// 结果用于核实请求放大；WebView 的渲染与输入时延由后续性能基线采集。
const folder = path.dirname(fileURLToPath(import.meta.url))
const project = path.resolve(folder, '../../..')
const require = createRequire(path.join(project, 'package.json'))
const vue = require('vue')
const { compileScript, parse } = require('vue/compiler-sfc')
const ts = require('typescript')
const route = vue.reactive({ name: 'xcloud', path: '/xcloud', fullPath: '/xcloud', params: {}, meta: { navPage: true } })
const locale = vue.ref('zh')
const listeners = new Map()
const calls = []
const modules = new Map()
let createdDetails = 0
let createdCatalogs = 0
let catalogSetup
const catalogTitles = ['PROBEA', 'PROBEB'].map(id => ({ id, productId: id, name: id, titleId: id, publisherName: '', description: '', categories: [], aliases: [], tileImageUrl: '', posterImageUrl: '', supportedInputTypes: [], hasEntitlement: false, isRecentlyPlayed: false, isNew: false }))
const payload = { scopeKey: 'probe', contentRevision: 'r0', updatedAt: 1, freshUntil: 600000, titles: catalogTitles, cacheState: 'fresh', refreshing: false, access: 'browse', sources: {} }
let activeDetails = 0

const events = {
  on(event, callback) {
    const group = listeners.get(event) ?? new Set()
    listeners.set(event, group)
    group.add(callback)
    return () => group.delete(callback)
  },
}
const rpc = {
  auth: { async getState() { return { isAuthenticated: false, isAuthenticating: false, appLevel: 0 } } },
  data: {
    async getXcloudTitles() {
      calls.push({ method: 'getXcloudTitles', route: route.path })
      return payload
    },
    async getCloudCatalogStatus() {
      calls.push({ method: 'getCloudCatalogStatus', route: route.path })
      const { titles, ...status } = payload
      return status
    },
    async getCloudTitleSummary({ productId }) {
      calls.push({ method: 'getCloudTitleSummary', productId, route: route.path })
      return { scopeKey: 'probe', title: catalogTitles.find(title => title.productId === productId) ?? null }
    },
    async prioritizeCloudTitles() {},
    async getCloudDetail({ productId }) {
      calls.push({ method: 'getCloudDetail', productId, route: route.path })
      return undefined
    },
    async getCloudPreferences() {
      return ['probe', { favorites: [], pinned: [], section: '', category: '', sort: 'name' }]
    },
    async cancelCloudWait() {},
  },
}

function evaluate(source, filename) {
  const code = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    fileName: filename,
  }).outputText
  const output = { exports: {} }
  runInNewContext(code, {
    module: output,
    exports: output.exports,
    require: name => resolve(name, filename),
    setTimeout,
    clearTimeout,
    crypto: globalThis.crypto,
    console,
    performance,
    requestAnimationFrame: callback => setTimeout(callback, 0),
    cancelAnimationFrame: clearTimeout,
    window: { innerWidth: 1440, addEventListener() {}, removeEventListener() {}, matchMedia: () => ({ matches: false }) },
  }, { filename })
  return output.exports
}
function resolve(name, importer) {
  if (name === 'vue') return vue
  if (name === 'vue-router') return { useRoute: () => route, useRouter: () => ({ push: async () => {} }), onBeforeRouteLeave() {} }
  if (name === 'vue-i18n') return { useI18n: () => ({ t: key => key, locale }) }
  if (name === '@/services/events') return { events }
  if (name === '@/services/rpc') return { rpc }
  if (name === '@/navigation/core/usePageFocus') return { usePageFocus: () => {
    const name = route.name
    return { save() {}, restore() {}, isActive: () => name === route.name }
  } }
  if (name === '@/navigation/core') return { navigationEngine: { onFocusChange: () => () => {}, onTabSwitch: () => () => {}, getCurrentFocus: () => null, isTopmostScope: () => true } }
  if (name === '@/navigation/core/haptics') return { playNavSound() {}, triggerNavHaptic() {} }
  if (name === '@/navigation/core/vue') return { Focusable: {} }
  if (name.endsWith('.vue') || name.endsWith('.scss')) return {}
  const target = name.startsWith('@/')
    ? path.join(project, 'src', name.slice(2))
    : path.resolve(path.dirname(importer), name)
  if (target.endsWith('/services/rpc')) return { rpc }
  if (target.endsWith('/services/events')) return { events }
  if (target.endsWith('/services/diagnostics')) return { diagnosticIssueId: () => undefined }
  if (target.endsWith('/shared/dev-log')) return { devWarn() {} }
  if (target.endsWith('/components/layout/shell-context')) return { useShellPageHints() {} }
  if (target.endsWith('/library/library-view')) return { formatMinutes: () => '' }
  if (target.endsWith('/library/use-library-query')) return { useLibraryQuery: () => ({ state: vue.ref({ value: null }) }) }
  const filename = `${target}.ts`
  if (!modules.has(filename)) modules.set(filename, evaluate(readFileSync(filename, 'utf8'), filename))
  return modules.get(filename)
}

const filename = path.join(project, 'src/pages/CloudGameDetail.vue')
const source = readFileSync(filename, 'utf8')
function component(filename) {
  const { descriptor } = parse(readFileSync(filename, 'utf8'))
  const compiled = compileScript(descriptor, { id: 'cloud-route-lifecycle-probe' })
  const component = evaluate(compiled.content, filename).default
  component.render = () => null
  return component
}
const Detail = component(filename)
const setup = Detail.setup
Detail.setup = (props, context) => {
  createdDetails++
  vue.onActivated(() => { activeDetails++ })
  vue.onDeactivated(() => { activeDetails-- })
  return setup(props, context)
}
const Catalog = component(path.join(project, 'src/pages/XCloud.vue'))
const setupCatalog = Catalog.setup
Catalog.setup = (props, context) => {
  createdCatalogs++
  catalogSetup = setupCatalog(props, context)
  return catalogSetup
}
// 用 App.vue 的实际路由 key 函数，覆盖固定详情实例与其他页面既有 key 合同。
const appSource = parse(readFileSync(path.join(project, 'src/App.vue'), 'utf8')).descriptor.scriptSetup.content
const ast = ts.createSourceFile('app.ts', appSource, ts.ScriptTarget.Latest, true)
const routeKeyNode = ast.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'resolveRouteViewKey')
const routeKey = evaluate(`export ${routeKeyNode.getText(ast)}`, 'route-key.ts').resolveRouteViewKey
const Other = { render: () => null }
const node = () => ({ parent: null, children: [] })
const detach = (element) => {
  if (element.parent) element.parent.children.splice(element.parent.children.indexOf(element), 1)
  element.parent = null
}
const renderer = vue.createRenderer({
  createElement: node,
  createText: node,
  createComment: node,
  setText() {},
  setElementText() {},
  patchProp() {},
  parentNode: element => element.parent,
  nextSibling: element => element.parent?.children[element.parent.children.indexOf(element) + 1] ?? null,
  insert(element, parent, anchor) {
    detach(element)
    const at = anchor ? parent.children.indexOf(anchor) : -1
    parent.children.splice(at < 0 ? parent.children.length : at, 0, element)
    element.parent = parent
  },
  remove: detach,
})
const app = renderer.createApp({
  render: () => vue.h(vue.KeepAlive, null, {
    default: () => route.name === 'cloud-game'
      ? vue.h(Detail, { key: routeKey(route) })
      : route.name === 'xcloud' ? vue.h(Catalog, { key: routeKey(route) }) : vue.h(Other, { key: routeKey(route) }),
  }),
})
async function settle() {
  for (let count = 0; count < 50; count++) await vue.nextTick()
}
const phases = []
let previousCalls = 0
function record(label) {
  const phaseCalls = calls.slice(previousCalls)
  phases.push({
    label,
    route: route.path,
    createdDetails,
    createdCatalogs,
    activeDetails,
    viewSubscribers: [...modules.values()].find(module => module.cloudCatalog)?.cloudCatalog.subscriberCount ?? 0,
    catalogListeners: listeners.get('data.xcloudCatalogUpdated')?.size ?? 0,
    catalogRequests: phaseCalls.filter(call => call.method === 'getXcloudTitles').length,
    statusRequests: phaseCalls.filter(call => call.method === 'getCloudCatalogStatus').length,
    summaryRequests: phaseCalls.filter(call => call.method === 'getCloudTitleSummary').map(call => call.productId),
    detailRequests: phaseCalls.filter(call => call.method === 'getCloudDetail').map(call => call.productId),
  })
  previousCalls = calls.length
}
app.mount(node())
await settle()
record('进入目录')
const firstIndex = catalogSetup.catalogIndex.value
const firstTitles = catalogSetup.titles.value
const steps = [
  ['打开详情 A', { name: 'cloud-game', path: '/xcloud/games/PROBEA', params: { productId: 'PROBEA' } }],
  ['回退目录', { name: 'xcloud', path: '/xcloud', params: {} }],
  ['打开详情 B', { name: 'cloud-game', path: '/xcloud/games/PROBEB', params: { productId: 'PROBEB' } }],
  ['切换设置', { name: 'setting', path: '/setting', params: {} }],
  ['返回目录', { name: 'xcloud', path: '/xcloud', params: {} }],
]
for (let loop = 0; loop < 20; loop++) {
  steps.push([`往返 ${loop + 1}: 详情`, { name: 'cloud-game', path: '/xcloud/games/PROBEA', params: { productId: 'PROBEA' } }])
  steps.push([`往返 ${loop + 1}: 目录`, { name: 'xcloud', path: '/xcloud', params: {} }])
  steps.push([`往返 ${loop + 1}: 设置`, { name: 'setting', path: '/setting', params: {} }])
  steps.push([`往返 ${loop + 1}: 目录`, { name: 'xcloud', path: '/xcloud', params: {} }])
}
for (const [label, next] of steps) {
  Object.assign(route, next, { fullPath: next.path })
  await settle()
  record(label)
}
const stableTitles = catalogSetup.titles.value === firstTitles
const stableIndex = catalogSetup.catalogIndex.value === firstIndex
app.unmount()
await settle()
assert.equal(createdDetails, 1)
assert.equal(createdCatalogs, 1)
assert.equal(calls.filter(call => call.method === 'getXcloudTitles').length, 1)
assert.equal(calls.filter(call => call.method === 'getCloudDetail' && !call.productId).length, 0)
assert.equal(phases.filter(phase => ['回退目录', '切换设置'].includes(phase.label)).flatMap(phase => phase.detailRequests).length, 0)
assert.equal(phases.some(phase => phase.activeDetails > 1 || phase.viewSubscribers > 1), false)
assert.equal(stableTitles, true)
assert.equal(stableIndex, true)
const result = {
  observedDate: '2026-11-12',
  source: {
    component: 'src/pages/CloudGameDetail.vue',
    sha256: createHash('sha256').update(source).digest('hex'),
    vueVersion: vue.version,
    nodeVersion: process.version,
    routeKeyEvidence: ['src/App.vue:56', 'src/router/index.ts:60'],
  },
  method: '实际 XCloud/CloudGameDetail setup + App 路由 key + Vue KeepAlive；RPC 受控响应；空渲染宿主',
  scope: '统计目录与详情请求、20 次往返生命周期和同版本索引复用；DOM/原生窗口时延另测',
  phases,
  totals: {
    createdDetails, createdCatalogs, stableTitles, stableIndex,
    statusRequests: calls.filter(call => call.method === 'getCloudCatalogStatus').length,
    summaryRequests: calls.filter(call => call.method === 'getCloudTitleSummary').length,
    catalogRequests: calls.filter(call => call.method === 'getXcloudTitles').length,
    detailRequests: calls.filter(call => call.method === 'getCloudDetail').length,
    emptyProductRequests: calls.filter(call => call.method === 'getCloudDetail' && call.productId === '').length,
    sharedCatalogOwnersAfterUnmount: listeners.get('data.xcloudCatalogUpdated')?.size ?? 0,
  },
}
writeFileSync(path.join(folder, 'route-lifecycle-result.json'), `${JSON.stringify(result, null, 2)}\n`)
process.stdout.write(`${JSON.stringify(result.totals, null, 2)}\n`)
