import type { PlaybackObservation, PlaybackTraceFacts } from '../playback-health'

export const facts: PlaybackTraceFacts = {
  lifecyclePhase: 'connected',
  expectedFps: 60,
  networkState: 'healthy',
  recoveryCause: 'unknown',
  firstFrameStage: 'firstPresented',
  presentationMilestone: 'mediaReady',
  controlChannelState: 'open',
  actionPending: false,
  visibility: 'visible',
  configuration: { requestedFps: 60, expectedFps: 60, rendererTargetFps: 60, sharpness: 3, requestedWidth: 2560, requestedHeight: 1440 },
  capabilities: { mediaBackend: 'browser', physicalPresent: 'unavailable', nativeRecoveryGate: 'unsupported', trackingSource: 'videoFrameCallback' },
}

export function observation(at: number, patch: Record<string, unknown> = {}, samplePatch: Record<string, unknown> = {}): PlaybackObservation {
  const frames = Math.round(at * 60 / 1000)
  return {
    sample: { sampleId: `1:${at}`, sampledAtMs: at, peerGeneration: 1, connectionState: 'connected', collectionDurationMs: 1, inboundVideo: { id: 'rtp', framesReceived: frames, framesDecoded: frames, packetsReceived: frames * 50, bytesReceived: frames * 40000, framesDropped: 0, freezeCount: 0, keyFramesDecoded: 1, pliCount: 0, firCount: 0, nackCount: 0 }, delta: { packetsLostDelta: 0, totalDecodeTimeDelta: 0.001 * 15 }, ...samplePatch },
    stats: { statsSampleId: `1:${at}`, statsObservationState: 'fresh', statsVideoGeneration: 1, rendererGeneration: 1, rendererKind: 'webgl2', rendererTargetFps: 60, videoPresentFramesTotal: frames, videoCallbacksTotal: frames, rendererSubmittedFramesTotal: frames, callbackGapCountTotal: 0, presentedFramesJumpCountTotal: 0, rendererLatestOverwriteCount: 0, rendererRateLimitDropCount: 0, rendererDuplicateDrawCount: 0, rendererSubmitFailureCount: 0, networkRttMs: 8, networkJitterMs: 2, jitterBufferMs: 3, renderSourceWidth: 2560, renderSourceHeight: 1440, renderActualOutputWidth: 2560, renderActualOutputHeight: 1440, inboundVideoFps: 60, decodeFps: 60, videoPresentFps: 60, callbackFps: 60, rendererSubmitFps: 60, ...patch },
  } as unknown as PlaybackObservation
}
