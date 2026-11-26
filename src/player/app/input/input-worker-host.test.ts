import type { GamepadFrame } from '../../domain/input'
import type { InputWorkerCommand, InputWorkerReply } from './input-worker-protocol'
import { neutralState } from '@shared/gamepad/state'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { InputWorkerHost } from './input-worker-host'
import { InputWorkerRuntime } from './input-worker-runtime'

class Channel extends EventTarget {
  readyState: RTCDataChannelState = 'open'
  bufferedAmount = 0
  bufferedAmountLowThreshold = 0
  binaryType = 'arraybuffer'
  send = vi.fn<(data: ArrayBuffer) => void>()
  close = vi.fn(() => { this.readyState = 'closed' })
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++)
    await Promise.resolve()
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

async function fixture(rejectTransfer = false) {
  vi.useFakeTimers()
  vi.setSystemTime(0)
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now())
  vi.stubGlobal('RTCDataChannel', Channel)
  class WorkerMock {
    static instance: WorkerMock
    onmessage?: (event: MessageEvent<InputWorkerReply>) => void
    onerror?: () => void
    onmessageerror?: () => void
    terminate = vi.fn()
    holdAck = false
    held: InputWorkerReply[] = []
    runtime = new InputWorkerRuntime({ postMessage: (message) => {
      if (this.holdAck && (message.type === 'framesAck' || message.type === 'barrierAck'))
        this.held.push(message)
      else
        queueMicrotask(() => this.onmessage?.({ data: message } as MessageEvent<InputWorkerReply>))
    } })

    postMessage = vi.fn((command: InputWorkerCommand, _transfer?: Transferable[]) => {
      if (command.type === 'attach' && rejectTransfer)
        throw new DOMException('unsupported transfer', 'DataCloneError')
      queueMicrotask(() => this.runtime.handle(command))
    })

    constructor() { WorkerMock.instance = this }
    release(): void {
      this.holdAck = false
      for (const message of this.held)
        this.onmessage?.({ data: message } as MessageEvent<InputWorkerReply>)
      this.held = []
    }
  }
  vi.stubGlobal('Worker', WorkerMock)
  const hooks = { trace: vi.fn(), packet: vi.fn(), rumble: vi.fn(), failed: vi.fn(), detached: vi.fn() }
  const host = (await InputWorkerHost.prepare({ preventResolutionDrops: false }, hooks))!
  expect(host).toBeDefined()
  const channel = new Channel()
  const frame = (down: boolean): GamepadFrame => {
    const state = neutralState()
    state.buttons.south = Number(down)
    return { gamepadIndex: 0, state, observedAtMs: Date.now() }
  }
  const close = async () => {
    host.close()
    await flush()
    expect(WorkerMock.instance.terminate).toHaveBeenCalledTimes(1)
    expect(vi.getTimerCount()).toBe(0)
  }
  return { host, channel, frame, hooks, worker: WorkerMock.instance, close }
}

describe('input Worker capability and bounded handoff', () => {
  it('transfers the actual unused channel and runs the accepted-send core in Worker', async () => {
    const f = await fixture()
    const binding = f.host.adopt(f.channel as unknown as RTCDataChannel)!
    expect(f.worker.postMessage).toHaveBeenCalledWith(expect.objectContaining({ type: 'attach', channel: f.channel }), [f.channel])
    await f.host.whenAttached()
    expect(binding.context.send(new ArrayBuffer(1))).toBe(false)
    f.host.start()
    f.host.queueFrame(f.frame(true), true)
    f.host.queueFrame(f.frame(false), true)
    await flush()
    vi.advanceTimersByTime(8)
    expect(f.channel.send.mock.calls.map(([data]) => new DataView(data)).filter(data => data.getUint16(0, true) & 2).map(data => data.getUint16(16, true))).toEqual([16, 0])
    await f.close()
    expect(f.hooks.failed).not.toHaveBeenCalled()
  })

  it('retains a usable Main channel when real transfer throws', async () => {
    const f = await fixture(true)
    expect(f.host.adopt(f.channel as unknown as RTCDataChannel)).toBeUndefined()
    expect(f.channel.readyState).toBe('open')
    expect(f.channel.close).not.toHaveBeenCalled()
    expect(f.worker.terminate).toHaveBeenCalledTimes(1)
    expect(f.hooks.trace).toHaveBeenCalledWith('inputExecutionCapabilityObserved', expect.objectContaining({ mode: 'main', reason: 'dataChannelTransferRejected' }))
    expect(vi.getTimerCount()).toBe(0)
  })

  it('bounds frame and barrier messages while an ACK is held and supersedes old-owner states', async () => {
    const f = await fixture()
    f.host.adopt(f.channel as unknown as RTCDataChannel)
    await f.host.whenAttached()
    f.host.start()
    f.worker.holdAck = true
    f.host.queueFrame(f.frame(true), true)
    await flush()
    f.host.queueFrame({ ...f.frame(false), resetReason: 'owner-changed' }, false)
    await flush()
    f.host.queueFrame({ ...f.frame(false), resetReason: 'device-changed' }, false)
    await flush()
    const commands = f.worker.postMessage.mock.calls.map(([command]) => command)
    expect(commands.filter(command => command.type === 'frames')).toHaveLength(1)
    expect(commands.filter(command => command.type === 'barrier')).toHaveLength(1)
    f.worker.release()
    await flush()
    vi.advanceTimersByTime(8)
    expect(f.channel.send.mock.calls.map(([data]) => new DataView(data)).filter(data => data.getUint16(0, true) & 2).every(data => data.getUint16(16, true) === 0)).toBe(true)
    await f.close()
  })
})
