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
