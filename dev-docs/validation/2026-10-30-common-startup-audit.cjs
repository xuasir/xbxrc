// 离线审查回放：断言 2026-10-30 观察到的缺陷行为；修复后应改用正式回归测试。
// node dev-docs/validation/2026-10-30-common-startup-audit.cjs
const fs = require('node:fs')
const path = require('node:path')
const os = require('node:os')
const { spawnSync } = require('node:child_process')

const root = path.resolve(__dirname, '../..')
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'xbxrc-startup-audit-'))
fs.symlinkSync(path.join(root, 'node_modules'), path.join(temporary, 'node_modules'))
function harness(file, boundary) {
  const absolute = path.join(root, file)
  const source = fs.readFileSync(absolute, 'utf8')
  const index = source.indexOf(boundary)
  if (index < 0) throw new Error(`找不到审查夹具边界: ${file}`)
  return source.slice(0, index).replace(/(['"])(\.\.?\/[^'"]+)\1/g,
    (_all, quote, relative) => `${quote}${path.resolve(path.dirname(absolute), relative)}${quote}`)
}

const hostTests = harness('src/streaming/runtime/runtime-host.test.ts', "describe('runtime host lifecycle'") + `
describe('公共启动审查：旧启动异常与 TURN 交接', () => {
  it.each(['during-stop', 'after-fallback-launch'])('%s: 旧启动异常撤销 TURN 尝试', async (timing) => {
    let rejectOld!: (error: Error) => void
    const oldLaunch = new Promise<void>((_resolve, reject) => { rejectOld = reject })
    const old = port()
    const next = port()
    old.launch.mockReturnValue(oldLaunch)
    if (timing === 'during-stop')
      old.stop.mockImplementation(async () => { rejectOld(new Error('streamRuntimeSuperseded')) })
    harness.factory.mockReset().mockReturnValueOnce(old).mockReturnValueOnce(next)
    const vm = host()
    const launchSpec = spec()
    launchSpec.turnSource = 'fallback'
    launchSpec.runtime.turnServer = { urls: ['turn:192.0.2.10:3478'] } as never
    const outcome = vm.startRuntime(launchSpec).then(() => 'resolved', error => error.message)
    await vi.advanceTimersByTimeAsync(500)
    old.subscribe.mock.calls[0][0]({ type: 'connectionStateChanged', state: 'failed' })
    await vi.advanceTimersByTimeAsync(200)
    if (timing === 'after-fallback-launch') {
      expect(next.launch).toHaveBeenCalledTimes(1)
      rejectOld(new Error('streamRuntimeSuperseded'))
      await vi.advanceTimersByTimeAsync(0)
    }
    expect(await outcome).toBe('streamRuntimeSuperseded')
    const traceEvents = harness.trace.mock.calls.map(([entry]) => entry.event)
    expect(traceEvents).toContain('fallbackTurnRetry')
    if (timing === 'during-stop') expect(next.launch).not.toHaveBeenCalled()
    else expect(next.stop).toHaveBeenCalledWith('launch-failed')
    console.log(JSON.stringify({ audit: timing, fallbackLaunches: next.launch.mock.calls.length,
      fallbackStops: next.stop.mock.calls, outcome: await outcome, traceEvents }))
    await vm.closeRuntime('audit-cleanup')
  })
  it('原生恢复 RPC 拒绝时产生未捕获异常，UI 未收到失败', async () => {
    const runtime = port()
    harness.factory.mockReset().mockReturnValue(runtime)
    const onState = vi.fn()
    const onError = vi.fn()
    const vm = useStreamRuntimeHost({ playerElementId: 'player', getSuperResolutionExperimental: () => false,
      onConnectionStateChange: onState, onRuntimeError: onError,
      onPresentationMilestoneChange: vi.fn(), onRuntimePhaseChange: vi.fn(), onFrameReady: vi.fn() })
    const launchSpec = spec()
    launchSpec.runtime.mode = 'rust-owned'
    const launch = vm.startRuntime(launchSpec)
    await vi.advanceTimersByTimeAsync(500)
    await launch
    const caught: unknown[] = []
    const observe = (error: unknown) => { caught.push(error) }
    process.on('unhandledRejection', observe)
    try {
      runtime.requestReconnect.mockRejectedValueOnce(new Error('AUDIT_NATIVE_RECONNECT_REJECTED'))
      runtime.subscribe.mock.calls[0][0]({ type: 'connectionStateChanged', state: 'failed' })
      await vi.advanceTimersByTimeAsync(0)
      expect(caught).toHaveLength(1)
      expect((caught[0] as Error).message).toBe('AUDIT_NATIVE_RECONNECT_REJECTED')
      expect(onState).not.toHaveBeenCalled()
      expect(onError).not.toHaveBeenCalled()
      expect(runtime.stop).not.toHaveBeenCalled()
      console.log(JSON.stringify({ audit: 'native-reconnect-rejection', unhandled: caught.length,
        uiStateUpdates: onState.mock.calls.length, uiErrors: onError.mock.calls.length,
        stopCalls: runtime.stop.mock.calls.length }))
    } finally {
      process.off('unhandledRejection', observe)
      await vm.closeRuntime('audit-cleanup')
    }
  })
})
`

let browserTests = harness('src/streaming/runtime/browser-runtime.test.ts', '\n  it(')
browserTests = browserTests.replace('readonly getPeer = vi.fn(() => undefined)',
  'readonly getPeer = vi.fn(() => (globalThis as any).auditPeer)')
browserTests += `
  it('仅收到 EOC 后结束协商，120 秒仍没有连接失败或恢复动作', async () => {
    vi.useFakeTimers()
    Object.assign(window, { setTimeout, clearTimeout, setInterval, clearInterval })
    class AuditPeer extends EventTarget {
      iceGatheringState = 'complete'
      iceConnectionState = 'new'
    }
    ;(globalThis as any).auditPeer = new AuditPeer()
    testState.rpc.streaming.pollIce.mockResolvedValue({ candidates: [
      { candidate: 'a=end-of-candidates', sdpMid: '0', sdpMLineIndex: 0 },
    ] } as never)
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    const events: any[] = []
    runtime.subscribe(event => events.push(event))
    const launch = runtime.launch(createLaunchSpec())
    await vi.advanceTimersByTimeAsync(100)
    await launch
    await vi.advanceTimersByTimeAsync(120_000)
    expect(testState.rpc.streaming.pollIce).toHaveBeenCalledTimes(1)
    expect(events.some(event => event.type === 'error')).toBe(false)
    expect(testState.rpc.streaming.decideRecovery).not.toHaveBeenCalled()
    expect(events.some(event => event.type === 'phaseChanged' && event.phase === 'connecting')).toBe(true)
    console.log(JSON.stringify({ audit: 'end-marker-only', elapsedMs: 120100,
      polls: testState.rpc.streaming.pollIce.mock.calls.length,
      recoveries: testState.rpc.streaming.decideRecovery.mock.calls.length,
      errors: events.filter(event => event.type === 'error').length,
      iceState: (globalThis as any).auditPeer.iceConnectionState }))
    await runtime.stop('audit-cleanup')
    delete (globalThis as any).auditPeer
    vi.clearAllTimers()
    vi.useRealTimers()
  })
})
`
fs.writeFileSync(path.join(temporary, 'host.test.ts'), hostTests)
fs.writeFileSync(path.join(temporary, 'browser.test.ts'), browserTests)
fs.writeFileSync(path.join(temporary, 'vite.config.mjs'), `export default {
  resolve: { alias: { '@': ${JSON.stringify(path.join(root, 'src'))}, '@shared': ${JSON.stringify(path.join(root, 'src/shared'))} } },
  define: { __APP_VERSION__: '"audit"' },
  // 仅过滤本回放显式计数断言的异常；其他未捕获异常照常使测试失败。
  test: { onUnhandledError(error) { if (error.message === 'AUDIT_NATIVE_RECONNECT_REJECTED') return false } },
}`)
console.log(`审查临时文件: ${temporary}`)
const result = spawnSync(path.join(root, 'node_modules/.bin/vitest'), ['run', '--root', temporary,
  '--config', path.join(temporary, 'vite.config.mjs'), '--reporter=verbose'], { cwd: root, stdio: 'inherit' })
process.exitCode = result.status ?? 1
