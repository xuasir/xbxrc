import { externalWebRtc } from './external-source'
import { installAblation } from './ablation'
import { createDrawableIdentitySampler } from './drawable-identity'
import { installDirectFrameProbe } from './direct-frames'
import { NativeVideoRenderer, WebGL2VideoRenderer } from '/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/Renderers'
import { SuperResolutionWebGL2Renderer } from '/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/SuperResolutionWebGL2Renderer'
import { getVideoFrameSource } from '/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-source'

const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
const finite = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? value : null
function summary(values: number[]) {
  const sorted = [...values].sort((a, b) => a - b)
  return { count: values.length, mean: values.reduce((a, b) => a + b, 0) / (values.length || 1), p50: sorted[Math.max(0, Math.ceil(values.length * .5) - 1)] ?? null, p95: sorted[Math.max(0, Math.ceil(values.length * .95) - 1)] ?? null, p99: sorted[Math.max(0, Math.ceil(values.length * .99) - 1)] ?? null, max: sorted.at(-1) ?? null }
}

async function waitFor(predicate: () => boolean, limitMs = 12000) {
  const started = performance.now()
  while (!predicate()) {
    if (performance.now() - started > limitMs) throw new Error('local fixture readiness timeout')
    await delay(50)
  }
}

async function localWebRtc(video: HTMLVideoElement) {
  if (new URLSearchParams(location.search).get('external') === '1') return externalWebRtc(video)
  const canvas = document.createElement('canvas')
  canvas.width = 2560
  canvas.height = 1440
  canvas.style.cssText = 'position:fixed;right:0;bottom:0;width:160px;height:90px;pointer-events:none'
  document.body.appendChild(canvas)
  const context = canvas.getContext('2d', { alpha: false })!
  let frames = 0
  let raf = 0
  const tick = () => {
    context.fillStyle = '#142234'
    context.fillRect(0, 0, canvas.width, canvas.height)
    context.fillStyle = frames % 2 ? '#ec6436' : '#287bcf'
    context.fillRect((frames * 16) % 2300, 500, 220, 220)
    context.fillStyle = '#fff'
    context.font = '72px monospace'
    context.fillText(String(frames++), 60, 120)
    raf = requestAnimationFrame(tick)
  }
  tick()
  const stream = canvas.captureStream(60)
  const sender = new RTCPeerConnection({ iceServers: [] })
  const receiver = new RTCPeerConnection({ iceServers: [] })
  let closing = false
  const candidateErrors: any[] = []
  const eventCounts = { senderCandidates: 0, receiverCandidates: 0, receivedTracks: 0, playFailures: 0 }
  const pendingSender: RTCIceCandidate[] = []
  const pendingReceiver: RTCIceCandidate[] = []
  const add = async (peer: RTCPeerConnection, candidate: RTCIceCandidate) => {
    try { await peer.addIceCandidate(candidate) }
    catch (error) { candidateErrors.push({ name: (error as Error).name }) }
  }
  sender.onicecandidate = event => {
    if (!event.candidate || closing) return
    eventCounts.senderCandidates++
    if (receiver.remoteDescription) void add(receiver, event.candidate)
    else pendingSender.push(event.candidate)
  }
  receiver.onicecandidate = event => {
    if (!event.candidate || closing) return
    eventCounts.receiverCandidates++
    if (sender.remoteDescription) void add(sender, event.candidate)
    else pendingReceiver.push(event.candidate)
  }
  receiver.ontrack = event => {
    eventCounts.receivedTracks++
    video.srcObject = new MediaStream([event.track])
    void video.play().catch(() => eventCounts.playFailures++)
  }
  const track = stream.getVideoTracks()[0]
  const settings = () => {
    const value = track.getSettings()
    return { width: value.width, height: value.height, frameRate: value.frameRate, resizeMode: value.resizeMode }
  }
  const sourceSetup: any = { initialTrackSettings: settings() }
  try { await track.applyConstraints({ width: { exact: 2560 }, height: { exact: 1440 }, frameRate: { ideal: 60 } }) }
  catch (error) { sourceSetup.constraintError = { name: (error as Error).name, message: (error as Error).message } }
  sourceSetup.constrainedTrackSettings = settings()
  const transceiver = sender.addTransceiver(track, { direction: 'sendonly', streams: [stream], sendEncodings: [{ maxBitrate: 12000000, maxFramerate: 60 }] })
  const codecs = RTCRtpSender.getCapabilities('video')!.codecs.filter(codec => codec.mimeType.toLowerCase() === 'video/h264')
  if (!codecs.length) throw new Error('H264 codec unavailable')
  transceiver.setCodecPreferences(codecs)
  await sender.setLocalDescription(await sender.createOffer())
  await receiver.setRemoteDescription(sender.localDescription!)
  for (const candidate of pendingSender.splice(0)) await add(receiver, candidate)
  await receiver.setLocalDescription(await receiver.createAnswer())
  await sender.setRemoteDescription(receiver.localDescription!)
  for (const candidate of pendingReceiver.splice(0)) await add(sender, candidate)
  try {
    const parameters = transceiver.sender.getParameters()
    parameters.degradationPreference = 'maintain-resolution'
    for (const encoding of parameters.encodings) encoding.scaleResolutionDownBy = 1
    await transceiver.sender.setParameters(parameters)
  }
  catch (error) { sourceSetup.senderParametersError = { name: (error as Error).name, message: (error as Error).message } }
  const applied = transceiver.sender.getParameters()
  sourceSetup.senderParameters = { degradationPreference: applied.degradationPreference, encodings: applied.encodings.map(encoding => ({ active: encoding.active, maxBitrate: encoding.maxBitrate, maxFramerate: encoding.maxFramerate, scaleResolutionDownBy: encoding.scaleResolutionDownBy })) }
  try { await waitFor(() => video.readyState >= 2 && video.videoWidth > 0) }
  catch (error) {
    const entries = [...(await receiver.getStats()).values()] as any[]
    const inbound = entries.find(entry => entry.type === 'inbound-rtp' && (entry.kind ?? entry.mediaType) === 'video')
    throw new Error(JSON.stringify({ phase: 'localWebRTC.firstDecodedVideo', error: String(error), senderConnection: sender.connectionState, receiverConnection: receiver.connectionState, senderIce: sender.iceConnectionState, receiverIce: receiver.iceConnectionState, senderGathering: sender.iceGatheringState, receiverGathering: receiver.iceGatheringState, eventCounts, candidateErrors, canvasFrames: frames, trackReady: track.readyState, trackMuted: track.muted, videoReady: video.readyState, videoPaused: video.paused, visibility: document.visibilityState, inbound: inbound ? { framesReceived: inbound.framesReceived, framesDecoded: inbound.framesDecoded, frameWidth: inbound.frameWidth, frameHeight: inbound.frameHeight, packetsReceived: inbound.packetsReceived } : null }))
  }
  await video.play()
  return {
    sourceSetup,
    senderFrames: () => frames,
    stats: async () => {
      const entries = [...(await receiver.getStats()).values()]
      const inbound: any = entries.find((entry: any) => entry.type === 'inbound-rtp' && (entry.kind ?? entry.mediaType) === 'video')
      const codec: any = entries.find((entry: any) => entry.type === 'codec' && entry.id === inbound?.codecId)
      if (!inbound) throw new Error('No inbound video statistics')
      return { timestamp: inbound.timestamp, framesReceived: inbound.framesReceived, framesDecoded: inbound.framesDecoded, framesDropped: inbound.framesDropped, packetsLost: inbound.packetsLost, nackCount: inbound.nackCount, freezeCount: inbound.freezeCount, totalFreezesDuration: inbound.totalFreezesDuration, totalDecodeTime: inbound.totalDecodeTime, frameWidth: inbound.frameWidth, frameHeight: inbound.frameHeight, codec: codec?.mimeType, fmtp: codec?.sdpFmtpLine }
    },
    close: () => { closing = true; cancelAnimationFrame(raf); track.stop(); sender.close(); receiver.close(); canvas.width = canvas.height = 1; canvas.remove() },
  }
}

async function measure(input: 'mp4' | 'webrtc', mode: 'native-only' | 'video' | 'sr-rgb' | 'sr-rgba') {
  const video = document.createElement('video')
  video.muted = true
  video.playsInline = true
  video.autoplay = true
  video.style.width = '800px'
  video.style.height = '450px'
  document.querySelector('#stage')!.appendChild(video)
  let rtc: Awaited<ReturnType<typeof localWebRtc>> | undefined
  if (input === 'mp4') {
    video.src = 'source.mp4'
    video.loop = true
    await video.play()
  }
  else {
    rtc = await localWebRtc(video)
    try { await waitFor(() => video.videoWidth === 2560 && video.videoHeight === 1440, 6000) }
    catch {}
    if (video.videoWidth !== 2560 || video.videoHeight !== 1440) {
      const diagnostics = { phase: 'localWebRTC.fixtureDimensions', videoWidth: video.videoWidth, videoHeight: video.videoHeight, sourceSetup: rtc.sourceSetup, stats: await rtc.stats() }
      rtc.close()
      throw new Error(JSON.stringify(diagnostics))
    }
  }
  const source = mode === 'native-only' ? undefined : getVideoFrameSource(video)
  source?.snapshotDiagnostics()
  const failures: string[] = []
  const config = { enabled: true, pipelineType: mode.startsWith('sr') ? 'webgl2_sr' : mode.startsWith('sharp') ? 'webgl2' : 'video', mode: 'native', processing: 'usm', processingMode: 'quality', sharpness: mode.startsWith('sharp') ? 3 : 0, targetFps: 60, format: 'Contain', brightness: 100, contrast: 100, saturation: 100, renderOutputWidth: 2560, renderOutputHeight: 1440, renderViewportWidth: 800, renderViewportHeight: 450, superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440, superResolutionRcasStops: .88, rendererRuntimeFailureNotifier: (reason: string) => failures.push(reason) } as const
  const renderer = mode.startsWith('sr') ? new SuperResolutionWebGL2Renderer(config) : mode.startsWith('sharp') ? new WebGL2VideoRenderer(config) : mode === 'video' ? new NativeVideoRenderer(config) : undefined
  await renderer?.attach(video)
  const processor = (renderer as any)?.processor ?? (renderer as any)?.player
  const ablation = undefined
  const direct = processor?.gl && new URLSearchParams(location.search).get('direct') === '1' ? await installDirectFrameProbe(video, renderer) : undefined
  const drawable = new URLSearchParams(location.search).get('drawable') === '1' ? createDrawableIdentitySampler(video) : undefined
  let driver: unknown
  if (processor?.gl) {
    const gl = processor.gl as WebGL2RenderingContext
    const extension = gl.getExtension('WEBGL_debug_renderer_info')
    driver = extension ? gl.getParameter(extension.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER)
    if (mode === 'sr-rgba') {
      const original = gl.texImage2D.bind(gl)
      ;(gl as any).texImage2D = (...args: any[]) => {
        if (args.length === 6 && args[5] === video && args[2] === gl.RGB) args[2] = args[3] = gl.RGBA
        return (original as any)(...args)
      }
    }
  }
  let request = 0
  let raf = 0
  let measuring = false
  let callbacks = 0
  let nativeSerial = 0
  let nativePresentedFrames: number | undefined
  let candidates = 0
  let advanced = 0
  let rafCount = 0
  let previousNative: any
  let previousRaf: any
  const markerStride = Math.max(1, Number(new URLSearchParams(location.search).get('markerStride') ?? 1))
  let markerTicks = 0
  const samples: any[] = []
  const intervals: number[] = []
  const leads: number[] = []
  const nativeDeltas: number[] = []
  const mediaDeltas: number[] = []
  const counterDeltas: number[] = []
  const counterIntervals: number[] = []
  const counterResets: any[] = []
  let lastCounterAt = 0
  const visibilityChanges: any[] = []
  const onVisibility = () => visibilityChanges.push({ at: performance.now(), state: document.visibilityState })
  document.addEventListener('visibilitychange', onVisibility)
  const read = () => {
    const quality = video.getVideoPlaybackQuality()
    return { at: performance.now(), currentTime: video.currentTime, visibility: document.visibilityState, paused: video.paused, readyState: video.readyState, qualityTotal: quality.totalVideoFrames, qualityDropped: quality.droppedVideoFrames, webkitTotal: finite((video as any).webkitDecodedFrameCount), webkitDropped: finite((video as any).webkitDroppedFrameCount) }
  }
  const subscribe = () => {
    const callback = (at: number, metadata: any) => {
      const now = performance.now()
      if (!source || metadata?.trackingSource === 'videoFrameCallback') {
        nativeSerial++
        nativePresentedFrames = metadata?.presentedFrames
      }
      if (measuring && (!source || metadata?.trackingSource === 'videoFrameCallback')) {
        callbacks++
        const row = { ...read(), callbackAt: at, enteredAt: now, presentedFrames: metadata?.presentedFrames, mediaTime: metadata?.mediaTime, presentationTime: metadata?.presentationTime, expectedDisplayTime: metadata?.expectedDisplayTime, rtpTimestamp: metadata?.rtpTimestamp, localSequence: metadata?.localSequence, newFrame: metadata?.newFrame, derivedAdvance: metadata?.presentedFramesDelta }
        if (previousNative) {
          intervals.push(now - previousNative.enteredAt)
          if (finite(row.presentedFrames) !== null && finite(previousNative.presentedFrames) !== null) nativeDeltas.push(row.presentedFrames - previousNative.presentedFrames)
          if (finite(row.mediaTime) !== null && finite(previousNative.mediaTime) !== null) mediaDeltas.push(row.mediaTime - previousNative.mediaTime)
        }
        if (finite(row.expectedDisplayTime) !== null) leads.push(row.expectedDisplayTime - now)
        previousNative = row
        if (samples.length < 24 || callbacks % 30 === 0) samples.push(row)
      }
      if (measuring) {
        candidates += Number(source ? metadata?.newFrame : true)
        advanced += metadata?.presentedFramesDelta ?? 0
      }
      request = subscribe()
    }
    return source ? source.request(callback) : video.requestVideoFrameCallback(callback)
  }
  request = subscribe()
  const onRaf = () => {
    const row = read()
    if (markerTicks++ % markerStride === 0)
      drawable?.sample({ nativeSerial, nativePresentedFrames, localSequence: source?.lastFrame?.localSequence ?? nativeSerial })
    if (measuring) {
      rafCount++
      if (previousRaf) {
        const delta = row.qualityTotal - previousRaf.qualityTotal
        if (delta > 0) { counterDeltas.push(delta); if (lastCounterAt) counterIntervals.push(row.at - lastCounterAt); lastCounterAt = row.at }
        if (delta < 0) counterResets.push({ before: previousRaf, after: row })
      }
      previousRaf = row
    }
    raf = requestAnimationFrame(onRaf)
  }
  raf = requestAnimationFrame(onRaf)
  await delay(2000)
  const start = { ...read(), frameSource: source?.snapshotResources(), direct: direct?.snapshot(), draw: renderer?.snapshotResources()?.realtimeDrawCount ?? 0, rtc: await rtc?.stats(), senderFrames: rtc?.senderFrames() }
  measuring = true
  await delay(Number(new URLSearchParams(location.search).get('durationMs') ?? 6500))
  measuring = false
  const end = { ...read(), frameSource: source?.snapshotResources(), direct: direct?.snapshot(), draw: renderer?.snapshotResources()?.realtimeDrawCount ?? 0, rtc: await rtc?.stats(), senderFrames: rtc?.senderFrames() }
  const elapsedMs = end.at - start.at
  const resources = renderer?.snapshotResources()
  const diagnostics = source?.snapshotDiagnostics()
  await drawable?.finish()
  if (failures.length || resources?.submitFailureCount) throw new Error(JSON.stringify({ failures, resources }))
  const foregroundValid = start.visibility === 'visible' && end.visibility === 'visible' && visibilityChanges.every(change => change.state === 'visible') && rafCount * 1000 / elapsedMs >= 50
  const result = { input, mode, markerStride, sourceSetup: rtc?.sourceSetup, foregroundValid, visibilityChanges, elapsedMs, callbackFps: callbacks * 1000 / elapsedMs, candidateFps: candidates * 1000 / elapsedMs, rafFps: rafCount * 1000 / elapsedMs, derivedAdvanceFps: advanced * 1000 / elapsedMs, rawNativeAdvanceFps: nativeDeltas.reduce((a, b) => a + b, 0) * 1000 / elapsedMs, rawMediaAdvanceFps: mediaDeltas.reduce((a, b) => a + b, 0) * 60 * 1000 / elapsedMs, drawFps: (mode.startsWith('sr') || mode.startsWith('sharp')) ? (end.draw - start.draw) * 1000 / elapsedMs : null, start, end, driver, callbackIntervalMs: summary(intervals), nativeDelta: summary(nativeDeltas), mediaDeltaSec: summary(mediaDeltas), expectedDisplayLeadMs: summary(leads), qualityUpdateDelta: summary(counterDeltas), qualityUpdateIntervalMs: summary(counterIntervals), counterResets, samples, resources, diagnostics, ablation: ablation?.snapshot(), drawableIdentity: drawable?.snapshot(start.at, end.at) }
  source ? source.cancel(request) : video.cancelVideoFrameCallback(request)
  cancelAnimationFrame(raf)
  document.removeEventListener('visibilitychange', onVisibility)
  ablation?.close()
  await direct?.close()
  drawable?.close()
  renderer?.destroy()
  ;(result as any).releasedFrameSource = source?.snapshotResources()
  rtc?.close()
  video.pause()
  video.srcObject = null
  video.removeAttribute('src')
  video.load()
  video.remove()
  await delay(200)
  return result
}

export async function run() {
  const runs = []
  const parameters = new URLSearchParams(location.search)
  const capabilities = typeof Worker !== 'function' ? { state: 'unavailable' } : await new Promise(resolve => {
    const script = new Blob([`postMessage({trackProcessor: typeof MediaStreamTrackProcessor, videoFrame: typeof VideoFrame, videoDecoder: typeof VideoDecoder, trackGenerator: typeof MediaStreamTrackGenerator})`], { type: 'text/javascript' })
    const workerUrl = URL.createObjectURL(script)
    const worker = new Worker(workerUrl)
    const done = (result: unknown) => { worker.terminate(); URL.revokeObjectURL(workerUrl); resolve(result) }
    const timer = setTimeout(() => done({ state: 'timeout' }), 1000)
    worker.onmessage = event => { clearTimeout(timer); done(event.data) }
    worker.onerror = () => { clearTimeout(timer); done({ state: 'error' }) }
  })
  const windowCapabilities = { trackProcessor: typeof (window as any).MediaStreamTrackProcessor, videoFrame: typeof VideoFrame, videoDecoder: typeof VideoDecoder, trackGenerator: typeof (window as any).MediaStreamTrackGenerator, worker: capabilities }
  if (parameters.get('modes') === 'capabilities')
    return { environment: { userAgent: navigator.userAgent, visible: document.visibilityState, capabilities: windowCapabilities }, runs }
  const modes = parameters.get('modes')?.split(',') ?? (parameters.get('external') === '1' ? ['native-only', 'video', 'sr-full', 'sharp-full', 'video'] : ['native-only', 'video', 'sr-full', 'sr-no-upload', 'sr-scissor', 'sr-transparent', 'sharp-full', 'sharp-no-upload', 'sharp-scissor', 'video'])
  for (const mode of modes) {
    document.querySelector('#status')!.textContent = `本地 1440p 对照：${mode}`
    const result = await measure('webrtc', mode as any)
    runs.push(result)
    const ms = result.end.at - result.start.at
    const progress = { mode, callbackFps: result.callbackFps, candidateFps: result.candidateFps, rafFps: result.rafFps, drawFps: result.drawFps,
      receivedFps: (result.end.rtc.framesReceived-result.start.rtc.framesReceived)*1000/ms,
      decodedFps: (result.end.rtc.framesDecoded-result.start.rtc.framesDecoded)*1000/ms,
      decodedFrameMode: result.end.frameSource?.renderFrameSource,
      decodedFramesFps: result.end.frameSource && result.start.frameSource ? (result.end.frameSource.decodedFramesReceived-result.start.frameSource.decodedFramesReceived)*1000/ms : null,
      width: result.end.rtc.frameWidth, height: result.end.rtc.frameHeight, foregroundValid: result.foregroundValid,
      uploadP95Ms: result.resources?.timing?.upload?.p95Ms, intervalP95Ms: result.resources?.timing?.interval?.p95Ms,
      uploads: result.ablation?.uploadCalls, skippedUploads: result.ablation?.skippedUploads,
      completedFenceSamples: result.ablation?.fence.samples.filter(s => s.state === 'completed').length,
      drawableChangeFps: result.drawableIdentity?.drawableChangeFps,
      drawableNewWithoutNative: result.drawableIdentity?.newImageWithoutNativeCallbackCount,
      drawableNewUnqualified: result.drawableIdentity?.newImageWithoutSourceQualificationCount,
      drawableValidCount: result.drawableIdentity?.validCount,
      drawableCpuP95Ms: result.drawableIdentity?.cpuP95Ms,
      fenceUpperBoundMs: result.ablation?.fence.samples.filter(s => s.state === 'completed').map(s => s.upperBoundMs) }
    await fetch('/progress', { method: 'POST', body: JSON.stringify(progress) })
  }
  return { environment: { userAgent: navigator.userAgent, visible: document.visibilityState, capabilities: windowCapabilities, input: 'local H264 WebRTC', width: 2560, height: 1440, targetFps: 60,
    qualification: 'source supply measured per run; fence latency includes queue and polling; no-upload/scissor are isolated diagnostic controls' }, runs }
}

;(window as any).XBXProbe = { run: async () => { try { return await run() } catch (error) { return { error: String(error), stack: (error as Error).stack } } } }
