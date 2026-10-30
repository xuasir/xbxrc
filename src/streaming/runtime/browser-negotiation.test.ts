import type { PlayerClient } from '../../player'
import type { RuntimeLaunchSpec } from '../types'
import type { StreamRuntimePhase } from './runtime-contract'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createOperationScope } from '../execution/async-operation'
import { createBrowserNegotiation } from './browser-negotiation'
import { createSerialOperationQueue } from './serial-operation'

type Candidate = Parameters<PlayerClient['addIceCandidates']>[0][number]
const rpc = vi.hoisted(() => ({
  streaming: { exchangeOffer: vi.fn(), submitIce: vi.fn(), pollIce: vi.fn() },
}))
vi.mock('../../services/rpc', () => ({ rpc }))

function candidate(address = '192.0.2.1'): Candidate {
  return { candidate: `candidate:1 1 udp 2130706431 ${address} 50000 typ host`, sdpMid: '0', sdpMLineIndex: 0 }
}
const end: Candidate = { candidate: 'a=end-of-candidates', sdpMid: '0', sdpMLineIndex: 0 }

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((yes) => {
    resolve = yes
  })
  return { promise, resolve }
}

class Peer extends EventTarget {
  iceGatheringState: RTCIceGatheringState = 'complete'
  iceConnectionState: RTCIceConnectionState = 'new'
  remoteDescription: { sdp: string } | null = null
  addEventListener = vi.fn(super.addEventListener.bind(this))
  removeEventListener = vi.fn(super.removeEventListener.bind(this))

  connect(state: 'connected' | 'completed' = 'connected') {
    this.iceConnectionState = state
    this.dispatchEvent(new Event('iceconnectionstatechange'))
  }

  gather(next: Candidate) {
    this.dispatchEvent(Object.assign(new Event('icecandidate'), { candidate: next }))
  }

  completeGathering() {
    this.iceGatheringState = 'complete'
    this.dispatchEvent(new Event('icegatheringstatechange'))
  }
}

function setup(gatheringState: RTCIceGatheringState = 'complete') {
  const peer = new Peer()
  peer.iceGatheringState = gatheringState
  const controller = new AbortController()
  const local = candidate('192.0.2.2')
  const client = {
    getTransportConfig: () => ({}),
    createOffer: vi.fn(async () => ({ type: 'offer', sdp: 'v=0\r\n' })),
    setRemoteDescription: vi.fn(async () => {}),
    getPeer: () => peer,
    getIceCandidates: () => [local],
    addIceCandidates: vi.fn(async (_candidates: Candidate[]) => {}),
  }
  const spec = { sessionId: 'session', targetType: 'home', runtime: {} } as RuntimeLaunchSpec
  const phases: StreamRuntimePhase[] = []
  const trace = vi.fn()
  const negotiation = createBrowserNegotiation({
    getClient: () => client as unknown as PlayerClient,
    getSpec: () => spec,
    isCurrentSpec: value => value === spec,
    getGeneration: () => 1,
    getSignal: () => controller.signal,
    getOfferPolicy: () => ({ patchEnabled: false, downshiftLevel: 0, baseVideoBitrateKbps: 10000 }),
    publishPhase: phase => phases.push(phase),
    trace,
    updateIcePolicy: vi.fn(),
    queue: createSerialOperationQueue(),
  })
  const start = () => negotiation.connectMediaProtocolNow(spec, { restart: false })
  return { peer, client, controller, spec, phases, trace, local, negotiation, start }
}

function expectListenersReleased(peer: Peer) {
  for (const [event, listener] of peer.addEventListener.mock.calls)
    expect(peer.removeEventListener).toHaveBeenCalledWith(event, listener)
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('window', { setTimeout, clearTimeout })
  vi.resetAllMocks()
  rpc.streaming.exchangeOffer.mockResolvedValue({ answer: { sdp: 'v=0\r\n' } })
  rpc.streaming.submitIce.mockResolvedValue({ accepted: true })
  rpc.streaming.pollIce.mockResolvedValue({ candidates: [] })
})
afterEach(() => {
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('browser remote ICE polling', () => {
  it('accepts an end marker when the current remote SDP already supplies candidates', async () => {
    const h = setup()
    h.peer.remoteDescription = { sdp: `v=0\r\na=${candidate().candidate}\r\n` }
    rpc.streaming.pollIce.mockResolvedValue({ candidates: [end] })
    const launch = h.start()
    await vi.advanceTimersByTimeAsync(60)
    await launch
    expect(h.client.addIceCandidates).toHaveBeenCalledExactlyOnceWith([end])
    expectListenersReleased(h.peer)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('fails promptly when the remote snapshot contains only an end marker', async () => {
    const h = setup()
    rpc.streaming.pollIce.mockResolvedValue({ candidates: [end] })
    const rejected = expect(h.start()).rejects.toThrow('remoteIceCandidatesMissing')
    await vi.advanceTimersByTimeAsync(60)
    await rejected
    expect(h.client.addIceCandidates).not.toHaveBeenCalled()
    expectListenersReleased(h.peer)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('waits through empty snapshots after local gathering completes and applies delayed candidates', async () => {
    const h = setup()
    rpc.streaming.pollIce.mockResolvedValueOnce({ candidates: [] }).mockResolvedValueOnce({ candidates: [] }).mockResolvedValueOnce({ candidates: [candidate(), end] })
    const finished = vi.fn()
    const launch = h.start().then(finished)
    await vi.advanceTimersByTimeAsync(1060)
    expect(rpc.streaming.pollIce).toHaveBeenCalledTimes(2)
    expect(finished).not.toHaveBeenCalled()
    expect(h.phases).not.toContain('connecting')
    await vi.advanceTimersByTimeAsync(1000)
    await launch
    expect(h.client.addIceCandidates).toHaveBeenCalledExactlyOnceWith([candidate(), end])
    expect(rpc.streaming.submitIce).toHaveBeenCalledExactlyOnceWith({ sessionId: 'session', candidate: [h.local], restart: false })
    expect(h.phases.at(-1)).toBe('connecting')
    expectListenersReleased(h.peer)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('applies additional remote batches once and keeps polling until the end marker', async () => {
    const h = setup()
    const second = candidate('192.0.2.3')
    rpc.streaming.pollIce.mockResolvedValueOnce({ candidates: [candidate()] }).mockResolvedValueOnce({ candidates: [candidate()] }).mockResolvedValueOnce({ candidates: [candidate(), second, end] })
    const launch = h.start()
    await vi.advanceTimersByTimeAsync(2060)
    await launch
    expect(h.client.addIceCandidates.mock.calls).toEqual([[[candidate()]], [[second, end]]])
    expect(rpc.streaming.pollIce).toHaveBeenCalledTimes(3)
    expect(vi.getTimerCount()).toBe(0)
  })

  it.each(['connected', 'completed'] as const)('finishes when ICE becomes %s without an end marker', async (state) => {
    const h = setup()
    rpc.streaming.pollIce.mockResolvedValue({ candidates: [candidate()] })
    const launch = h.start()
    await vi.advanceTimersByTimeAsync(60)
    h.peer.connect(state)
    await launch
    await vi.advanceTimersByTimeAsync(2000)
    expect(rpc.streaming.pollIce).toHaveBeenCalledTimes(1)
    expectListenersReleased(h.peer)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('submits late local candidates while waiting for remote readiness', async () => {
    const h = setup('gathering')
    const late = candidate('192.0.2.4')
    const launch = h.start()
    await vi.advanceTimersByTimeAsync(60)
    rpc.streaming.pollIce.mockResolvedValue({ candidates: [candidate(), end] })
    h.peer.gather(late)
    h.peer.completeGathering()
    await vi.advanceTimersByTimeAsync(60)
    await launch
    expect(rpc.streaming.submitIce.mock.calls.map(([params]) => params.candidate)).toEqual([[h.local], [late]])
    expect(h.client.addIceCandidates).toHaveBeenCalledExactlyOnceWith([candidate(), end])
  })

  it('exchanges fresh candidates on restart while the peer still reports the old connection', async () => {
    const h = setup()
    h.peer.iceConnectionState = 'connected'
    rpc.streaming.pollIce.mockResolvedValueOnce({ candidates: [] }).mockResolvedValueOnce({ candidates: [candidate()] })
    const finished = vi.fn()
    const launch = h.negotiation.connectMediaProtocolNow(h.spec, { restart: true }).then(finished)
    await vi.advanceTimersByTimeAsync(60)
    expect(finished).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1000)
    await launch
    expect(rpc.streaming.pollIce).toHaveBeenCalledTimes(2)
    expect(h.client.addIceCandidates).toHaveBeenCalledExactlyOnceWith([candidate()])
    expect(h.client.createOffer).toHaveBeenCalledWith(expect.objectContaining({ iceRestart: true }))
    expect(vi.getTimerCount()).toBe(0)
  })

  it('drains local candidates after the remote end marker', async () => {
    const h = setup('gathering')
    rpc.streaming.pollIce.mockResolvedValue({ candidates: [candidate(), end] })
    const finished = vi.fn()
    const launch = h.start().then(finished)
    await vi.advanceTimersByTimeAsync(60)
    expect(finished).not.toHaveBeenCalled()
    const late = candidate('192.0.2.4')
    h.peer.gather(late)
    h.peer.completeGathering()
    await vi.advanceTimersByTimeAsync(60)
    await launch
    expect(rpc.streaming.submitIce.mock.calls.map(([params]) => params.candidate)).toEqual([[h.local], [late]])
    expect(rpc.streaming.pollIce).toHaveBeenCalledTimes(1)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('bounds persistent empty responses by the existing 45 second deadline', async () => {
    const h = setup()
    const rejected = expect(h.start()).rejects.toThrow('iceExchangeTimeout')
    await vi.advanceTimersByTimeAsync(45000)
    await rejected
    expect(rpc.streaming.pollIce).toHaveBeenCalledTimes(45)
    expect(h.client.addIceCandidates).not.toHaveBeenCalled()
    expectListenersReleased(h.peer)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('cancels the scheduled poll and removes listeners on exit', async () => {
    const h = setup()
    const rejected = expect(h.start()).rejects.toThrow('streamRuntimeSuperseded')
    await vi.advanceTimersByTimeAsync(60)
    h.controller.abort()
    await rejected
    await vi.advanceTimersByTimeAsync(2000)
    expect(rpc.streaming.pollIce).toHaveBeenCalledTimes(1)
    expectListenersReleased(h.peer)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('keeps one poll in flight and absorbs its late result after cancellation', async () => {
    const h = setup()
    const response = deferred<{ candidates: Candidate[] }>()
    rpc.streaming.pollIce.mockReturnValue(response.promise)
    const rejected = expect(h.start()).rejects.toThrow('streamRuntimeSuperseded')
    await vi.advanceTimersByTimeAsync(3060)
    expect(rpc.streaming.pollIce).toHaveBeenCalledTimes(1)
    h.controller.abort()
    await rejected
    response.resolve({ candidates: [candidate(), end] })
    await vi.advanceTimersByTimeAsync(2000)
    expect(h.client.addIceCandidates).not.toHaveBeenCalled()
    expect(h.phases).not.toContain('connecting')
    expectListenersReleased(h.peer)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('honors the parent recovery deadline while waiting for remote candidates', async () => {
    const h = setup()
    const scope = createOperationScope(500, 'parentDeadline')
    const launch = h.negotiation.connectMediaProtocolNow(h.spec, {
      restart: true,
      operation: { ...scope, isCurrent: () => !scope.signal.aborted } as never,
    })
    const rejected = expect(launch).rejects.toThrow('parentDeadline')
    await vi.advanceTimersByTimeAsync(500)
    await rejected
    scope.dispose()
    expect(rpc.streaming.pollIce).toHaveBeenCalledTimes(1)
    expectListenersReleased(h.peer)
    expect(vi.getTimerCount()).toBe(0)
  })
})
