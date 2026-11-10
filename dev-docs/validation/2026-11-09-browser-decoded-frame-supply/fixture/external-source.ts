export async function externalWebRtc(video: HTMLVideoElement) {
  const offer = await (await fetch('/rtc/offer', { method: 'POST' })).json()
  const receiver = new RTCPeerConnection({ iceServers: [] })
  const playErrors: string[] = []
  receiver.ontrack = event => {
    if (event.track.kind === 'video') {
      video.srcObject = new MediaStream([event.track])
      void video.play().catch(error => playErrors.push(String(error)))
    }
  }
  await receiver.setRemoteDescription(offer.description)
  await receiver.setLocalDescription(await receiver.createAnswer())
  await new Promise<void>((resolve, reject) => {
    if (receiver.iceGatheringState === 'complete') { resolve(); return }
    const timer = setTimeout(() => reject(new Error('receiver ICE gather timeout')), 10000)
    const changed = () => {
      if (receiver.iceGatheringState === 'complete') { clearTimeout(timer); receiver.removeEventListener('icegatheringstatechange', changed); resolve() }
    }
    receiver.addEventListener('icegatheringstatechange', changed)
  })
  await fetch('/rtc/answer', { method: 'POST', body: JSON.stringify({ id: offer.id, description: receiver.localDescription }) })
  const limit = performance.now() + 12000
  while (video.readyState < 2 || video.videoWidth !== 2560 || video.videoHeight !== 1440) {
    if (performance.now() > limit) {
      const entries: any[] = [...(await receiver.getStats()).values()]
      const inbound = entries.find(entry => entry.type === 'inbound-rtp' && (entry.kind ?? entry.mediaType) === 'video')
      const sender = await (await fetch('/rtc/debug', { method: 'POST', body: JSON.stringify({ id: offer.id }) })).json()
      throw new Error(JSON.stringify({ phase: 'external1440p.readiness', connection: receiver.connectionState, ice: receiver.iceConnectionState, playErrors,
        video: { readyState: video.readyState, paused: video.paused, width: video.videoWidth, height: video.videoHeight, currentTime: video.currentTime }, sourceSetup: offer.sourceSetup,
        inbound: inbound ? { framesReceived: inbound.framesReceived, framesDecoded: inbound.framesDecoded, framesDropped: inbound.framesDropped, frameWidth: inbound.frameWidth, frameHeight: inbound.frameHeight, packetsReceived: inbound.packetsReceived } : null, sender }))
    }
    await new Promise(resolve => setTimeout(resolve, 50))
  }
  return {
    sourceSetup: offer.sourceSetup,
    senderFrames: () => 0,
    stats: async () => {
      const entries = [...(await receiver.getStats()).values()]
      const inbound: any = entries.find((entry: any) => entry.type === 'inbound-rtp' && (entry.kind ?? entry.mediaType) === 'video')
      const codec: any = entries.find((entry: any) => entry.type === 'codec' && entry.id === inbound?.codecId)
      if (!inbound) throw new Error('No external video statistics')
      const sender = await (await fetch('/rtc/debug', { method: 'POST', body: JSON.stringify({ id: offer.id }) })).json()
      return { timestamp: inbound.timestamp, framesReceived: inbound.framesReceived, framesDecoded: inbound.framesDecoded, framesDropped: inbound.framesDropped,
        packetsLost: inbound.packetsLost, nackCount: inbound.nackCount, freezeCount: inbound.freezeCount, totalFreezesDuration: inbound.totalFreezesDuration,
        totalDecodeTime: inbound.totalDecodeTime, frameWidth: inbound.frameWidth, frameHeight: inbound.frameHeight, codec: codec?.mimeType, fmtp: codec?.sdpFmtpLine,
        jitter: inbound.jitter, jitterBufferDelay: inbound.jitterBufferDelay, jitterBufferEmittedCount: inbound.jitterBufferEmittedCount, sender }
    },
    close: () => { receiver.close(); void fetch('/rtc/close', { method: 'POST', body: JSON.stringify({ id: offer.id }) }) },
  }
}
