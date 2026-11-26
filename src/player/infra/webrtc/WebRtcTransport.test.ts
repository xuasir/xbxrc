import { afterEach, expect, it, vi } from 'vitest'
import { WebRtcTransport } from './WebRtcTransport'

afterEach(() => vi.unstubAllGlobals())

it('forwards end-of-candidates to the peer with its media binding', async () => {
  const addIceCandidate = vi.fn(async () => {})
  vi.stubGlobal('RTCPeerConnection', class extends EventTarget {
    addIceCandidate = addIceCandidate
    addTransceiver = vi.fn()
    getTransceivers = () => []
  })
  const transport = new WebRtcTransport()
  transport.bind()
  await transport.addIceCandidate({ candidate: 'a=end-of-candidates', sdpMid: 'video', sdpMLineIndex: 1 })
  expect(addIceCandidate).toHaveBeenCalledExactlyOnceWith({ candidate: '', sdpMid: 'video', sdpMLineIndex: 1 })
})

it('retires a captured peer after the input barrier while a new peer stays active', async () => {
  const peers: Array<{ close: ReturnType<typeof vi.fn> }> = []
  vi.stubGlobal('RTCPeerConnection', class extends EventTarget {
    close = vi.fn()
    addTransceiver = vi.fn()
    getTransceivers = () => []
    constructor() {
      super()
      peers.push(this)
    }
  })
  const transport = new WebRtcTransport()
  transport.bind()
  let complete!: () => void
  const barrier = new Promise<void>((resolve) => {
    complete = resolve
  })
  transport.close(barrier)
  expect(peers[0]!.close).not.toHaveBeenCalled()
  const current = transport.bind()
  complete()
  await barrier
  expect(peers[0]!.close).toHaveBeenCalledTimes(1)
  expect(peers[1]!.close).not.toHaveBeenCalled()
  expect(transport.getPeer()).toBe(current)
  transport.close()
})

it('disowns a transferred channel before Window cleanup uses its methods', () => {
  const channel = { close: vi.fn(() => {
    throw new Error('detached channel')
  }) }
  const close = vi.fn()
  vi.stubGlobal('RTCPeerConnection', class extends EventTarget {
    close = close
    addTransceiver = vi.fn()
    getTransceivers = () => []
    createDataChannel = () => channel
  })
  const transport = new WebRtcTransport()
  transport.bind()
  const created = transport.createDataChannel('input', {})
  transport.releaseDataChannel('input', created)
  transport.close()
  expect(channel.close).not.toHaveBeenCalled()
  expect(close).toHaveBeenCalledTimes(1)
})
