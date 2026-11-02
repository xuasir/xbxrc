import type { PlayerEvents } from '../../api/events'
import type { GamepadFrame } from '../../domain/input'
import type { MediaService } from '../media/MediaService'
import { inputController } from '@shared/gamepad/runtime'
import { neutralState } from '@shared/gamepad/state'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TypedEventEmitter } from '../../api/events'
import { DEFAULT_PLAYER_OPTIONS } from '../../domain/config'
import { InputService } from '../input/InputService'
import { SessionService } from './SessionService'

vi.mock('@shared/gamepad/runtime', async () => {
  const { InputController } = await import('@shared/gamepad/input-controller')
  return { inputController: new InputController() }
})
vi.mock('../../../services/rpc', () => ({ rpc: { runtimeTrace: { recordEvent: async () => {} } } }))

class FakeDataChannel extends EventTarget {
  readyState: RTCDataChannelState = 'open'
  bufferedAmount = 0
  send = vi.fn<(data: string | ArrayBuffer) => void>()
  close = vi.fn(() => {
    if (this.readyState === 'closed')
      return
    this.readyState = 'closed'
    this.dispatchEvent(new Event('close'))
  })

  handshake(): void {
    this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ type: 'HandshakeAck' }) }))
  }
}

class FakePeer extends EventTarget {
  static instances: FakePeer[] = []
  readonly channels = new Map<string, FakeDataChannel>()
  connectionState: RTCPeerConnectionState = 'new'
  signalingState = 'stable'
  iceConnectionState = 'new'
  iceGatheringState = 'new'
  ontrack: ((event: RTCTrackEvent) => void) | null = null
  addTransceiver = vi.fn()
  getTransceivers = () => []
  getReceivers = () => []
  close = vi.fn(() => {
    this.connectionState = 'closed'
    for (const channel of this.channels.values())
      channel.close()
    this.dispatchEvent(new Event('connectionstatechange'))
  })

  constructor() {
    super()
    FakePeer.instances.push(this)
  }

  createDataChannel(name: string): FakeDataChannel {
    const channel = new FakeDataChannel()
    this.channels.set(name, channel)
    return channel
  }

  changeState(state: RTCPeerConnectionState): void {
    this.connectionState = state
    this.dispatchEvent(new Event('connectionstatechange'))
  }
}

const sessions = new Set<SessionService>()
beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('window', globalThis)
  vi.stubGlobal('RTCPeerConnection', FakePeer)
  FakePeer.instances = []
})
afterEach(async () => {
  for (const session of sessions)
    session.close()
  sessions.clear()
  await inputController.stop()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

function fixture() {
  const emitter = new TypedEventEmitter<PlayerEvents>()
  const driver = {
    start: vi.fn(),
    stop: vi.fn(),
    requestStates: (): GamepadFrame[] => [{ gamepadIndex: 0, state: neutralState() }],
  }
  const input = new InputService(driver, emitter)
  const rumble = vi.spyOn(input, 'handleRumble')
  const media = { destroy: vi.fn(), attachVideoStream: vi.fn(), attachAudioStream: vi.fn() }
  const session = new SessionService(DEFAULT_PLAYER_OPTIONS(), input, media as unknown as MediaService, emitter)
  sessions.add(session)
  const timeline = vi.fn()
  const candidates = vi.fn()
  emitter.on('stats.browserWebRtcTimeline', timeline)
  emitter.on('transport.iceCandidate', candidates)
  return { session, input, driver, rumble, media, timeline, candidates }
}

function dispatchLateEvents(peer: FakePeer): void {
  peer.channels.get('message')!.handshake()
  for (const name of ['input', 'control']) {
    peer.channels.get(name)!.dispatchEvent(new Event('open'))
    peer.channels.get(name)!.dispatchEvent(new Event('close'))
  }
  peer.channels.get('input')!.dispatchEvent(new MessageEvent('message', { data: new ArrayBuffer(13) }))
  peer.ontrack?.({ streams: [{}], track: { kind: 'video' } } as unknown as RTCTrackEvent)
  peer.dispatchEvent(Object.assign(new Event('icecandidate'), {
    candidate: { candidate: 'old-candidate', sdpMid: 'video', sdpMLineIndex: 1 },
  }))
  peer.changeState('failed')
  for (const name of ['signalingstatechange', 'iceconnectionstatechange', 'icegatheringstatechange'])
    peer.dispatchEvent(new Event(name))
}

describe('browser session replacement', () => {
  it('isolates input, handshake, rumble, tracks and ICE from a failed peer after rebinding', () => {
    const f = fixture()
    f.session.bind()
    const previous = FakePeer.instances[0]
    previous.channels.get('message')!.handshake()
    previous.channels.get('input')!.bufferedAmount = 18 * 1024
    const pressed = neutralState()
    pressed.buttons.south = 1
    f.input.queueGamepadState({ gamepadIndex: 0, state: pressed })
    f.input.addProcessedFrame({ serverDataKey: 77, firstFramePacketArrivalTimeMs: 1, frameSubmittedTimeMs: 2, frameDecodedTimeMs: 3, frameRenderedTimeMs: 4 })
    previous.changeState('failed')
    f.session.bind()
    const current = FakePeer.instances[1]
    current.channels.get('message')!.handshake()
    current.changeState('connected')
    expect(previous.close).toHaveBeenCalledOnce()
    expect([...previous.channels.values()].every(channel => channel.readyState === 'closed')).toBe(true)
    const stopCount = f.driver.stop.mock.calls.length
    const timelineCount = f.timeline.mock.calls.length
    dispatchLateEvents(previous)
    expect(f.driver.start).toHaveBeenCalledTimes(2)
    expect(f.driver.stop).toHaveBeenCalledTimes(stopCount)
    expect(f.rumble).not.toHaveBeenCalled()
    expect(f.media.attachVideoStream).not.toHaveBeenCalled()
    expect(f.candidates).not.toHaveBeenCalled()
    expect(f.session.getIceCandidates()).toEqual([])
    expect(f.timeline).toHaveBeenCalledTimes(timelineCount)
    expect(f.session.getState()).toBe('connected')
    f.input.queueGamepadState({ gamepadIndex: 0, state: pressed })
    const sent = current.channels.get('input')!.send.mock.calls.map(([data]) => new DataView(data as ArrayBuffer))
    expect(sent.map(packet => packet.getUint32(2, true))).toEqual([0, 1])
    expect(sent[1].getUint16(0, true)).toBe(2)
    expect(sent[1].getUint16(16, true)).toBe(16)
    current.channels.get('input')!.dispatchEvent(new MessageEvent('message', { data: new ArrayBuffer(13) }))
    expect(f.rumble).toHaveBeenCalledOnce()
    expect(vi.getTimerCount()).toBe(2)
  })

  it('reclaims retry and control timers on close and keeps the next session alive after delayed events', () => {
    const f = fixture()
    f.session.bind()
    const previous = FakePeer.instances[0]
    previous.channels.get('message')!.handshake()
    expect(vi.getTimerCount()).toBe(2)
    f.session.close()
    expect(vi.getTimerCount()).toBe(0)
    f.session.bind()
    const current = FakePeer.instances[1]
    current.channels.get('message')!.handshake()
    dispatchLateEvents(previous)
    vi.advanceTimersByTime(500)
    expect(f.driver.start).toHaveBeenCalledTimes(2)
    expect(current.channels.get('control')!.send).toHaveBeenCalledWith(JSON.stringify({ message: 'gamepadChanged', gamepadIndex: 0, wasAdded: true }))
    expect(current.channels.get('input')!.send.mock.calls.length).toBeGreaterThan(1)
    f.session.close()
    f.session.close()
    expect(vi.getTimerCount()).toBe(0)
  })
})
