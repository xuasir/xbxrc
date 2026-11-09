import { afterEach, describe, expect, it, vi } from 'vitest'
import { inputUiTrace } from './input-trace'

let dispose: (() => void) | undefined
afterEach(() => {
  dispose?.()
  vi.restoreAllMocks()
})
function fixture() {
  let at = 0
  vi.spyOn(performance, 'now').mockImplementation(() => at)
  const rows: Array<{ event: string, payload: any }> = []
  const write = vi.fn((event: string, payload: Record<string, unknown>) => {
    rows.push({
      event,
      payload,
    })
    return true
  })
  const input = () => ({ owner: 'ui', backend: 'browser', activeDeviceId: 'private-controller', exclusiveCapture: false, sampling: { sampledAtMs: at, pollingHz: 300, publishedHz: 40, gap: { retained: 256, p95Ms: 4, p99Ms: 10, maxMs: 25 } } })
  const anomaly = vi.fn()
  dispose = inputUiTrace.attach({ enabled: () => true, write, input, anomaly })
  inputUiTrace.scope('stream.action-sheet', true)
  inputUiTrace.navigationState('stream.action-sheet')
  return {
    rows,
    write,
    anomaly,
    time: (next: number) => {
      at = next
    },
  }
}

describe('input result evidence', () => {
  it('记录 busy 重复确认并合并诊断，正常等待保持输入健康', () => {
    const f = fixture()
    const action = vi.fn()
    inputUiTrace.activation('gamepad', 'fullscreen', 'busy', action)
    inputUiTrace.activation('gamepad', 'fullscreen', 'busy', action)
    expect(action).not.toHaveBeenCalled()
    expect(f.rows.filter(row => row.payload.reason === 'busy')).toHaveLength(1)
    expect(f.rows.at(-1)?.payload).toMatchObject({ source: 'gamepad', outcome: 'blocked', reason: 'busy' })
    expect(f.anomaly).not.toHaveBeenCalled()
  })

  it('异步操作入口的 busy 结果继续配对，等待期间保持输入健康', () => {
    const f = fixture()
    const finish = inputUiTrace.beginOperation('text')
    finish('blocked', 'busy')
    expect(f.rows.at(-1)?.payload).toMatchObject({ action: 'text', outcome: 'blocked', reason: 'busy' })
    expect(f.anomaly).not.toHaveBeenCalled()
  })

  it('pairs source/operation across handler invocation and async completion after teardown', async () => {
    const f = fixture()
    let finish: (result: string) => void = () => {}
    inputUiTrace.withIntent('action', 'gamepad', () => inputUiTrace.activation('mouse', 'fullscreen', undefined, () => {
      finish = inputUiTrace.beginOperation('fullscreen')
    }))
    const invoked = f.rows.find(row => row.payload.outcome === 'handlerInvoked')!
    dispose?.()
    f.time(100)
    finish('operationCompleted')
    expect(f.rows.at(-1)?.payload).toMatchObject({ source: 'gamepad', action: 'fullscreen', operationId: invoked.payload.operationId, outcome: 'operationCompleted', durationMs: 100 })
  })

  it('records a touch ancestor-scope block before any handler and redacts dynamic targets', () => {
    const f = fixture()
    const action = vi.fn()
    inputUiTrace.activation('touch', 'private-user-name', 'inactiveAncestorScope', action)
    expect(action).not.toHaveBeenCalled()
    expect(f.rows.at(-1)?.payload).toMatchObject({ source: 'touch', outcome: 'blocked', reason: 'inactiveAncestorScope', action: 'other' })
    expect(JSON.stringify(f.rows)).not.toContain('private')
    expect(f.anomaly).toHaveBeenCalledWith('inputFailed')
  })

  it('coalesces sustained navigation into health under row and minute budgets', () => {
    const f = fixture()
    for (let second = 1; second <= 120; second++) {
      f.time(second * 1000)
      for (let i = 0; i < 1000; i++) inputUiTrace.withIntent('down', 'gamepad', () => inputUiTrace.navigationResult('down', true))
      inputUiTrace.tick()
    }
    const health = f.rows.filter(row => row.event === 'inputUiHealthObserved')
    expect(health).toHaveLength(120)
    expect(health.every(row => JSON.stringify(row).length + 300 <= 1024)).toBe(true)
    expect(health.at(-1)?.payload).toMatchObject({ pollingHz: 300, publishedHz: 40, directions: { down: 1000 }, intentCount: 120000 })
    expect(f.rows.reduce((sum, row) => sum + JSON.stringify(row).length + 300, 0) / 2).toBeLessThanOrEqual(65536)
  })
})
