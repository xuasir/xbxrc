const video = document.querySelector('video')
let current
const canvasSource = new URLSearchParams(location.search).get('producer') === 'canvas'
const jitterOffsets = new URLSearchParams(location.search).get('jitter') === '1' ? [0, 6, -6, 4, -4, 7, -7, 3, -3, 0] : [0]
let canvas, frameCount = 0, nextAt = performance.now(), producerTimer
if (canvasSource) {
  video.pause()
  video.removeAttribute('src')
  video.load()
  canvas = document.createElement('canvas')
  canvas.width = 2560
  canvas.height = 1440
  canvas.style.cssText = 'width:800px;height:450px'
  document.body.appendChild(canvas)
  const context = canvas.getContext('2d', { alpha: false })
  context.fillStyle = '#142234'
  context.fillRect(0, 0, 2560, 1440)
  const tick = () => {
    context.fillStyle = '#142234'
    context.fillRect(0, 0, 2560, 360)
    context.fillStyle = '#fff'
    context.font = '72px monospace'
    context.fillText(String(frameCount), 60, 120)
    context.fillStyle = frameCount % 2 ? '#ec6436' : '#287bcf'
    context.fillRect((frameCount * 13) % 2100, 150, 240, 160)
    for (let bit = 0; bit < 32; bit++) {
      const value = bit < 24 ? (frameCount >> bit) & 1 : (0xd3 >> (bit - 24)) & 1
      context.fillStyle = value ? '#f0f0f0' : '#101010'
      context.fillRect(bit * 32, 0, 32, 64)
    }
    frameCount++
    current?.track.requestFrame?.()
    nextAt += 1000 / 60
    producerTimer = setTimeout(tick, Math.max(0, nextAt + jitterOffsets[frameCount % jitterOffsets.length] - performance.now()))
  }
  tick()
}
const ready = (async () => {
  if (canvasSource) return
  await video.play()
  while (video.readyState < 2 || video.videoWidth !== 2560 || video.videoHeight !== 1440)
    await new Promise(resolve => setTimeout(resolve, 20))
})()
async function gather(peer) {
  if (peer.iceGatheringState === 'complete') return
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('sender ICE gather timeout')), 10000)
    const changed = () => { if (peer.iceGatheringState === 'complete') { clearTimeout(timer); peer.removeEventListener('icegatheringstatechange', changed); resolve() } }
    peer.addEventListener('icegatheringstatechange', changed)
  })
}
window.XBXSender = {
  offer: async (id) => {
    await ready
    if (current) { current.peer.close(); current.track.stop() }
    const stream = canvasSource ? canvas.captureStream(0) : video.captureStream()
    const track = stream.getVideoTracks()[0]
    const peer = new RTCPeerConnection({ iceServers: [] })
    const transceiver = peer.addTransceiver(track, { direction: 'sendonly', streams: [stream], sendEncodings: [{ maxBitrate: 35000000, maxFramerate: 60, scaleResolutionDownBy: 1 }] })
    transceiver.setCodecPreferences(RTCRtpSender.getCapabilities('video').codecs.filter(codec => codec.mimeType.toLowerCase() === 'video/h264'))
    await peer.setLocalDescription(await peer.createOffer())
    await gather(peer)
    current = { id, peer, track, transceiver }
    const settings = track.getSettings()
    return { description: { type: peer.localDescription.type, sdp: peer.localDescription.sdp }, sourceSetup: { producer: canvasSource ? 'independent Chromium timer-paced 1440p/60 canvas H264' : 'independent Chromium 1440p/60 H264 file capture', width: settings.width, height: settings.height, frameRate: settings.frameRate, jitterOffsetsMs: jitterOffsets } }
  },
  answer: async (id, description) => {
    if (current?.id !== id) throw new Error('stale local peer answer')
    await current.peer.setRemoteDescription(description)
    const parameters = current.transceiver.sender.getParameters()
    parameters.degradationPreference = 'maintain-resolution'
    for (const encoding of parameters.encodings) encoding.scaleResolutionDownBy = 1
    await current.transceiver.sender.setParameters(parameters)
    return { completed: true }
  },
  debug: async (id) => {
    const entries = current?.id === id ? [...(await current.peer.getStats()).values()] : []
    const outbound = entries.find(entry => entry.type === 'outbound-rtp' && entry.kind === 'video')
    const source = entries.find(entry => entry.type === 'media-source' && entry.kind === 'video')
    return { producerFrameCount: frameCount, video: { readyState: video.readyState, paused: video.paused, width: video.videoWidth, height: video.videoHeight, quality: video.getVideoPlaybackQuality(), currentTime: video.currentTime, visibility: document.visibilityState },
      track: current ? { width: current.track.getSettings().width, height: current.track.getSettings().height, frameRate: current.track.getSettings().frameRate } : null, connection: current?.peer.connectionState, ice: current?.peer.iceConnectionState,
      parameters: current?.transceiver.sender.getParameters().degradationPreference,
      outbound: outbound ? { framesSent: outbound.framesSent, framesEncoded: outbound.framesEncoded, frameWidth: outbound.frameWidth, frameHeight: outbound.frameHeight, framesPerSecond: outbound.framesPerSecond, totalEncodeTime: outbound.totalEncodeTime, packetsSent: outbound.packetsSent, qualityLimitationReason: outbound.qualityLimitationReason, encoderImplementation: outbound.encoderImplementation } : null,
      source: source ? { frames: source.frames, framesPerSecond: source.framesPerSecond, width: source.width, height: source.height } : null }
  },
  close: (id) => {
    if (current?.id === id) { current.peer.close(); current.track.stop(); current = undefined }
    return { completed: true }
  },
}
