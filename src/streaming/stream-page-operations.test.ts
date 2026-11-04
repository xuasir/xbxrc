import { describe, expect, it, vi } from 'vitest'
import { useStreamPageOperations } from './stream-page-operations'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}
describe('stream page operation feedback', () => {
  it('keeps one operation in flight and reports rejected actions without exposing errors', async () => {
    const request = deferred<boolean>()
    const perform = vi.fn(() => request.promise)
    const actions = useStreamPageOperations(key => key)
    const first = actions.run('text', perform)
    await actions.run('text', perform)
    expect(perform).toHaveBeenCalledTimes(1)
    expect(actions.pending.text).toBe(true)
    request.resolve(false)
    expect(await first).toBe(false)
    expect(actions.errors.text).toBe('streamPage.operations.text')
    expect(actions.pending.text).toBe(false)
    expect(await actions.run('text', async () => true)).toBe(true)
    expect(actions.errors.text).toBeUndefined()
  })

  it('invalidates a closed panel and absorbs old completions while a new operation runs', async () => {
    const old = deferred<void>()
    const current = deferred<void>()
    const actions = useStreamPageOperations(key => key)
    const oldSuccess = vi.fn()
    const first = actions.run('display', () => old.promise, oldSuccess)
    actions.invalidate('display')
    const second = actions.run('display', () => current.promise)
    old.reject(new Error('private-account'))
    await first
    expect(actions.errors.display).toBeUndefined()
    expect(actions.pending.display).toBe(true)
    expect(oldSuccess).toHaveBeenCalledTimes(0)
    current.resolve()
    await second
    expect(actions.pending.display).toBe(false)
  })

  it('revokes continuation eligibility and separates navigation failure from shutdown failure', async () => {
    const waiting = deferred<void>()
    const actions = useStreamPageOperations(key => key)
    let current!: () => boolean
    const run = actions.run('export', async (isCurrent) => {
      current = isCurrent
      await waiting.promise
    })
    expect(current()).toBe(true)
    actions.invalidate('export')
    expect(current()).toBe(false)
    waiting.resolve()
    expect(await run).toBe(false)
    await actions.run('powerOff', async () => {
      throw new Error('streamNavigationFailed')
    })
    expect(actions.errorCodes.powerOff).toBe('navigationFailed')
    expect(actions.errors.powerOff).toBe('streamPage.operations.exit')
    await actions.run('powerOff', async () => {
      throw new Error('powerOffRejected')
    })
    expect(actions.errorCodes.powerOff).toBeUndefined()
    expect(actions.errors.powerOff).toBe('streamPage.operations.powerOff')
  })
})
