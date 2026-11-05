import { describe, expect, it, vi } from 'vitest'
import { GpuFrameTimer } from './gpu-frame-timer'

function fixture() {
  let disjoint = false
  let available = false
  const gl = {
    QUERY_RESULT_AVAILABLE: 1,
    QUERY_RESULT: 2,
    getExtension: vi.fn(() => ({ TIME_ELAPSED_EXT: 3, GPU_DISJOINT_EXT: 4 })),
    getParameter: vi.fn(() => disjoint),
    getQueryParameter: vi.fn((_query, key) => key === 1 ? available : 2_500_000),
    createQuery: vi.fn(() => ({})),
    beginQuery: vi.fn(),
    endQuery: vi.fn(),
    deleteQuery: vi.fn(),
  }
  return { gl, timer: new GpuFrameTimer(gl as unknown as WebGL2RenderingContext), available: () => {
    available = true
  }, disjoint: () => {
    disjoint = true
  } }
}

describe('asynchronous GPU timing', () => {
  it('keeps one query in flight and reads only available results at one Hz', () => {
    const f = fixture()
    f.timer.begin(0)
    f.timer.end()
    for (let i = 1; i < 1000; i++) f.timer.begin(i)
    expect(f.gl.getQueryParameter).not.toHaveBeenCalled()
    f.timer.begin(1000)
    expect(f.gl.createQuery).toHaveBeenCalledTimes(1)
    f.available()
    f.timer.begin(2000)
    f.timer.end()
    expect(f.timer.snapshot()).toMatchObject({ queryCount: 1, duration: { count: 1, p95Ms: 2.5 } })
    f.timer.dispose()
    expect(f.gl.deleteQuery).toHaveBeenCalledTimes(2)
    expect(f.timer.snapshot().queryCount).toBe(0)
  })

  it('invalidates disjoint evidence and falls back when the extension is absent', () => {
    const f = fixture()
    f.timer.begin(0)
    f.timer.end()
    f.available()
    f.timer.begin(1000)
    f.timer.end()
    f.disjoint()
    f.timer.begin(2000)
    expect(f.timer.snapshot()).toMatchObject({ state: 'disjoint', queryCount: 0, duration: { count: 0 }, disjointCount: 1 })
    const unavailable = new GpuFrameTimer({ getExtension: () => null } as unknown as WebGL2RenderingContext)
    unavailable.begin(0)
    unavailable.end()
    expect(unavailable.snapshot()).toMatchObject({ state: 'unavailable', queryCount: 0 })
  })

  it('releases expired queries and isolates driver failures from rendering', () => {
    const f = fixture()
    f.timer.begin(0)
    f.timer.end()
    f.timer.begin(5000)
    expect(f.timer.snapshot()).toMatchObject({ state: 'failed', queryCount: 0 })
    expect(f.gl.deleteQuery).toHaveBeenCalledTimes(1)
    const failed = fixture()
    failed.gl.beginQuery.mockImplementation(() => {
      throw new Error('contextLost')
    })
    failed.gl.deleteQuery.mockImplementation(() => {
      throw new Error('driverFailed')
    })
    expect(() => failed.timer.begin(0)).not.toThrow()
    expect(() => failed.timer.dispose()).not.toThrow()
    expect(failed.timer.snapshot()).toMatchObject({ state: 'failed', queryCount: 0 })
  })

  it('clears prior evidence and the pending query when ending a sample fails', () => {
    const f = fixture()
    f.timer.begin(0)
    f.timer.end()
    f.available()
    f.timer.begin(1000)
    expect(f.timer.snapshot().duration.count).toBe(1)
    f.gl.endQuery.mockImplementation(() => {
      throw new Error('contextLost')
    })
    expect(() => f.timer.end()).not.toThrow()
    expect(f.timer.snapshot()).toMatchObject({ state: 'failed', queryCount: 0, sampledAtMs: undefined, duration: { count: 0 } })
    expect(f.gl.deleteQuery).toHaveBeenCalledTimes(2)
    f.timer.begin(2000)
    expect(f.gl.createQuery).toHaveBeenCalledTimes(2)
  })

  it('disables timing when query allocation fails', () => {
    const f = fixture()
    f.gl.createQuery.mockReturnValue(null as unknown as WebGLQuery)
    f.timer.begin(0)
    expect(f.timer.snapshot()).toMatchObject({ state: 'failed', queryCount: 0, duration: { count: 0 } })
    expect(f.gl.beginQuery).not.toHaveBeenCalled()
  })
})
