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
const route = vue.reactive({ name: 'cloud-game', path: '/xcloud/games/PROBE-A', params: { productId: 'PROBE-A' } })
const locale = vue.ref('zh')
const listeners = new Map()
const calls = []
const modules = new Map()
let createdDetails = 0
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
  data: {
    async getXcloudTitles() {
      calls.push({ method: 'getXcloudTitles', route: route.path })
      return { scopeKey: 'probe', updatedAt: 1, titles: [], cacheState: 'fresh', refreshing: false }
    },
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
  }, { filename })
  return output.exports
}
function resolve(name, importer) {
  if (name === 'vue') return vue
  if (name === 'vue-router') return { useRoute: () => route, useRouter: () => ({ push: async () => {} }) }
  if (name === 'vue-i18n') return { useI18n: () => ({ t: key => key, locale }) }
  if (name === '@/services/events') return { events }
  if (name === '@/services/rpc') return { rpc }
  if (name === '@/navigation/core/usePageFocus') return { usePageFocus: () => {} }
  if (name === '@/navigation/core/vue') return { Focusable: {} }
  if (name.endsWith('.vue') || name.endsWith('.scss')) return {}
  const target = name.startsWith('@/')
    ? path.join(project, 'src', name.slice(2))
    : path.resolve(path.dirname(importer), name)
  if (target.endsWith('/library/library-view')) return { formatMinutes: () => '' }
  if (target.endsWith('/library/use-library-query')) return { useLibraryQuery: () => ({ state: vue.ref({ value: null }) }) }
  const filename = `${target}.ts`
  if (!modules.has(filename)) modules.set(filename, evaluate(readFileSync(filename, 'utf8'), filename))
  return modules.get(filename)
}

const filename = path.join(project, 'src/pages/CloudGameDetail.vue')
const source = readFileSync(filename, 'utf8')
const { descriptor } = parse(source)
const compiled = compileScript(descriptor, { id: 'cloud-route-lifecycle-probe' })
const Detail = evaluate(compiled.content, filename).default
const setup = Detail.setup
Detail.setup = (props, context) => {
  createdDetails++
  vue.onActivated(() => { activeDetails++ })
  vue.onDeactivated(() => { activeDetails-- })
  return setup(props, context)
}
Detail.render = () => null
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
      ? vue.h(Detail, { key: route.path })
      : vue.h(Other, { key: route.name }),
  }),
})
async function settle() {
  for (let count = 0; count < 12; count++) await vue.nextTick()
}
const phases = []
let previousCalls = 0
function record(label) {
  const phaseCalls = calls.slice(previousCalls)
  phases.push({
    label,
    route: route.path,
    createdDetails,
    activeDetails,
    catalogListeners: listeners.get('data.xcloudCatalogUpdated')?.size ?? 0,
    catalogRequests: phaseCalls.filter(call => call.method === 'getXcloudTitles').length,
    detailRequests: phaseCalls.filter(call => call.method === 'getCloudDetail').map(call => call.productId),
  })
  previousCalls = calls.length
}
app.mount(node())
await settle()
record('打开详情 A')
for (const [label, next] of [
  ['回退目录', { name: 'xcloud', path: '/xcloud', params: {} }],
  ['打开详情 B', { name: 'cloud-game', path: '/xcloud/games/PROBE-B', params: { productId: 'PROBE-B' } }],
  ['切换设置', { name: 'setting', path: '/setting', params: {} }],
]) {
  Object.assign(route, next)
  await settle()
  record(label)
}
app.unmount()
await settle()
assert.deepEqual(phases.map(phase => phase.catalogRequests), [1, 1, 2, 2])
assert.deepEqual(phases.map(phase => phase.activeDetails), [1, 0, 1, 0])
assert.equal(listeners.get('data.xcloudCatalogUpdated')?.size, 0)
const result = {
  observedDate: '2026-11-12',
  source: {
    component: 'src/pages/CloudGameDetail.vue',
    sha256: createHash('sha256').update(source).digest('hex'),
    vueVersion: vue.version,
    nodeVersion: process.version,
    routeKeyEvidence: ['src/App.vue:56', 'src/router/index.ts:60'],
  },
  method: '实际 CloudGameDetail.vue setup + Vue KeepAlive；RPC 受控响应；空渲染宿主',
  scope: '统计详情路由请求次数与订阅生命周期；XCloud 页面自身查询和原生窗口时延单独测量',
  phases,
  totals: {
    catalogRequests: calls.filter(call => call.method === 'getXcloudTitles').length,
    detailRequests: calls.filter(call => call.method === 'getCloudDetail').length,
    emptyProductRequests: calls.filter(call => call.method === 'getCloudDetail' && call.productId === '').length,
    catalogListenersAfterUnmount: listeners.get('data.xcloudCatalogUpdated')?.size ?? 0,
  },
}
writeFileSync(path.join(folder, 'route-lifecycle-result.json'), `${JSON.stringify(result, null, 2)}\n`)
process.stdout.write(`${JSON.stringify(result, null, 2)}\n`)
