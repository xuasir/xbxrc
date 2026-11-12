import { router } from '/src/router/index.ts'
import { rpc } from '/src/services/rpc.ts'

export function readHomeEvidence() {
  const primary = document.querySelector('.home-page__primary')
  return {
    title: document.querySelector('.home-page__console-name')?.textContent.trim() ?? '',
    status: document.querySelector('.home-page__status')?.textContent.trim() ?? '',
    deviceCards: document.querySelectorAll('.home-page__device-grid .console-status-card').length,
    primaryBusy: primary?.getAttribute('aria-busy') ?? '',
    primaryDisabled: primary?.disabled ?? null,
    loading: !!document.querySelector('.home-page__loading'),
    unavailable: document.querySelector('.home-page .page-unavailable')?.textContent.trim() ?? '',
    scrollTop: document.querySelector('.home-page')?.scrollTop ?? 0,
  }
}

export function createHostsFixture({ emit, record, counters }) {
  const originalNow = Date.now.bind(Date)
  let clockOffset = 0
  Date.now = () => originalNow() + clockOffset
  let account = 'fixture-account-a'
  let delay = 80
  let failure = false
  const initial = [
    { id: 'fixture-console', deviceId: 'fixture-device', serverId: 'fixture-server', name: '客厅 Xbox', consoleType: 'Series X', powerState: 'On', consoleStreamingEnabled: true, remoteManagementEnabled: true },
    { id: 'fixture-console-s', deviceId: 'fixture-device-s', serverId: 'fixture-server-s', name: '书房 Xbox', consoleType: 'Series S', powerState: 'ConnectedStandby', consoleStreamingEnabled: true, remoteManagementEnabled: true },
  ]
  const sources = new Map([
    ['fixture-account-a', structuredClone(initial)],
    ['fixture-account-b', [{ ...initial[0], id: 'fixture-console-b', serverId: 'fixture-server-b', name: '账户 B 的 Xbox' }]],
  ])
  const cache = new Map()
  const flights = new Map()
  const generations = new Map()
  const wait = ms => new Promise(resolve => setTimeout(resolve, ms))
  const scope = () => account
  const state = () => ({ account, delay, failure, clockOffset, cacheScopes: [...cache.keys()], inflightScopes: [...flights.keys()], networkRequests: counters['host.http'] ?? 0 })

  function invalidate(key) {
    generations.set(key, (generations.get(key) ?? 0) + 1)
    if (cache.has(key)) cache.get(key).freshUntil = 0
  }
  async function refresh() {
    const key = scope()
    if (flights.has(key)) return flights.get(key)
    counters['host.http'] = (counters['host.http'] ?? 0) + 1
    const generation = generations.get(key) ?? 0
    const hosts = structuredClone(sources.get(key))
    const promise = (async () => {
      await wait(delay)
      if (key !== scope()) throw new Error('登录状态已更新，请重新查询')
      if (generation !== (generations.get(key) ?? 0)) throw new Error('主机状态已更新，请重新查询')
      if (failure) throw new Error('主机刷新失败，保留最近成功内容')
      const snapshot = { scopeKey: key, hosts, updatedAt: Date.now(), freshUntil: Date.now() + 60_000 }
      cache.set(key, snapshot)
      return structuredClone(snapshot)
    })().finally(() => { flights.delete(key); record() })
    flights.set(key, promise)
    record()
    return promise
  }
  async function invoke(method, params) {
    if (method === 'getHostsSnapshot') return cache.has(scope()) ? structuredClone(cache.get(scope())) : refresh()
    if (method === 'refreshHosts') return refresh()
    if (method === 'getHosts') {
      const result = await refresh()
      emit('xbxrc:data:hosts-changed', { scopeKey: result.scopeKey })
      return result.hosts
    }
    if (method === 'powerOnConsole' || method === 'powerOffConsole') {
      const host = sources.get(scope()).find(host => host.id === params.consoleId)
      if (host) host.powerState = method === 'powerOnConsole' ? 'On' : 'Off'
      invalidate(scope())
      emit('xbxrc:data:hosts-changed', { scopeKey: scope() })
      return { consoleId: params.consoleId, accepted: true }
    }
  }

  const controls = document.createElement('details')
  controls.id = 'home-validation-controls'
  controls.style.cssText = 'position:fixed;right:12px;bottom:42px;z-index:10001;background:#20282a;color:white;padding:6px;border:1px solid #61726c;border-radius:6px;font:12px sans-serif;max-width:300px'
  const buttons = [
    ['roundtrips', '主机与设置往返 ×20'], ['expire', '推进 61 秒并使缓存到期'],
    ['slow', '慢请求 3 秒'], ['fast', '快速请求'], ['cold', '清空缓存并冷加载'],
    ['fail', '启用刷新失败'], ['recover', '恢复主机服务'], ['empty', '服务端空列表'], ['restore', '恢复两台主机'],
    ['account', '切换账户'], ['session', '同账户会话就绪'], ['notify', '主机状态变更通知'], ['foreign', '旧账户变更通知'],
    ['power', '接受书房开机命令'], ['record', '记录主机状态'],
  ]
  controls.innerHTML = `<summary>主机验证操作</summary><div style="display:grid;grid-template-columns:1fr 1fr;gap:6px;margin-top:8px">${buttons.map(([id, label]) => `<button type="button" id="home-validation-${id}">${label}</button>`).join('')}</div><output id="home-validation-output" style="display:block;max-width:280px;white-space:pre-wrap"></output>`
  document.body.append(controls)
  const output = controls.querySelector('output')
  controls.addEventListener('pointerdown', event => { if (event.target instanceof HTMLButtonElement) event.preventDefault() })
  controls.addEventListener('click', async event => {
    const button = event.target.closest('button')
    if (!button) return
    output.dataset.running = 'true'
    try {
      const action = button.id.replace('home-validation-', '')
      if (action === 'roundtrips') {
        const before = { ...counters }
        for (let i = 0; i < 20; i++) {
          await router.push('/setting'); await wait(180); record()
          await router.push('/xhome'); await wait(180); record()
        }
        output.textContent = JSON.stringify({ action, iterations: 20, snapshotDelta: (counters['data.getHostsSnapshot'] ?? 0) - (before['data.getHostsSnapshot'] ?? 0), refreshDelta: (counters['data.refreshHosts'] ?? 0) - (before['data.refreshHosts'] ?? 0), networkDelta: (counters['host.http'] ?? 0) - (before['host.http'] ?? 0) })
      } else {
        if (action === 'expire') { clockOffset += 61_000; for (const key of cache.keys()) invalidate(key) }
        if (action === 'slow') delay = 3_000
        if (action === 'fast') delay = 80
        if (action === 'cold') { cache.clear(); emit('xbxrc:auth:session-ready', { provider: 'xal', appLevel: 2, at: new Date().toISOString() }) }
        if (action === 'fail') failure = true
        if (action === 'recover') failure = false
        if (action === 'empty') sources.set(account, [])
        if (action === 'restore') sources.set(account, structuredClone(initial))
        if (action === 'account') { account = account === 'fixture-account-a' ? 'fixture-account-b' : 'fixture-account-a'; emit('xbxrc:auth:session-ready', { provider: 'xal', appLevel: 2, at: new Date().toISOString() }) }
        if (action === 'session') emit('xbxrc:auth:session-ready', { provider: 'xal', appLevel: 2, at: new Date().toISOString() })
        if (action === 'notify') { invalidate(account); emit('xbxrc:data:hosts-changed', { scopeKey: account }) }
        if (action === 'foreign') emit('xbxrc:data:hosts-changed', { scopeKey: account === 'fixture-account-a' ? 'fixture-account-b' : 'fixture-account-a' })
        if (action === 'power') await rpc.data.powerOnConsole({ consoleId: 'fixture-console-s' })
        output.textContent = JSON.stringify({ action, ...state() })
      }
      record()
    } catch (error) { output.textContent = error.message }
    finally { output.dataset.running = 'false'; record() }
  })
  return { invoke, state }
}
