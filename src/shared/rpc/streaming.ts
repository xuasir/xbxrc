export type StreamingTargetType = 'home' | 'cloud'
export type StreamingStartupPhaseStatus = 'entered' | 'succeeded' | 'failed'
export type StreamingStartupBoundedRetryStatus = 'retrying' | 'exhausted'
export type StreamingStartupBoundedRetryReason = 'waitingForServerRegistration'
export type StreamingStartupPhase
  = | 'resolvingContext'
    | 'creatingSession'
    | 'waitingSessionReady'
    | 'startingRuntime'
    | 'ready'
    | 'failed'
export type StreamingStartupErrorKind
  = | 'sessionCreate'
    | 'sessionReady'
    | 'runtime'
    | 'network'
    | 'auth'
    | 'target'
    | 'hostRemotePlayUnavailable'
    | 'hostRegistrationRetryExhausted'
    | 'unknown'

export interface StreamingStartupBoundedRetry {
  reason: StreamingStartupBoundedRetryReason
  status: StreamingStartupBoundedRetryStatus
  retryCount: number
  retryLimit: number
}

export interface StreamingStartupEvent {
  attemptId: string
  targetType: StreamingTargetType
  targetId: string
  phase: StreamingStartupPhase
  status: StreamingStartupPhaseStatus
  summary: string
  details?: string
  boundedRetry?: StreamingStartupBoundedRetry | null
  tsMs: number
}

export interface StreamingStartupError {
  attemptId: string
  phase: StreamingStartupPhase
  errorKind: StreamingStartupErrorKind
  userMessageKey: string
  diagnosticSummary: string
  rawMessage: string
  retryable: boolean
  boundedRetry?: StreamingStartupBoundedRetry | null
}

export interface StreamingSessionError {
  errorKind: StreamingStartupErrorKind
  userMessageKey: string
  diagnosticSummary: string
  rawMessage: string
  retryable: boolean
  boundedRetry?: StreamingStartupBoundedRetry | null
}

export type StreamingPlayerState = 'pending' | 'started' | 'queued' | 'failed'

export type StreamingStreamState
  = | 'Provisioning'
    | 'Provisioned'
    | 'ReadyToConnect'
    | 'WaitingForResources'
    | 'Failed'
    | (string & {})

export type StreamingErrorCode = string | number

export interface StreamingErrorDetails {
  code?: StreamingErrorCode
  message?: string
}

export interface StreamingQueueDetails {
  estimatedTotalWaitTimeInSeconds?: number
  estimatedAllocationTimeInSeconds?: number
  estimatedProvisioningTimeInSeconds?: number
}

export interface StreamingQueueSnapshot {
  details: StreamingQueueDetails
}

export interface StreamingAnswerPayload {
  sdp: string
  messageType?: string
}

export interface StreamingIceCandidate {
  candidate: string
  sdpMLineIndex?: number | null
  sdpMid?: string | null
  usernameFragment?: string | null
  messageType?: string
}

export interface StreamingTurnServerConfig {
  url: string
  username: string
  credential: string
}

export interface StreamingRuntimeCodecPreference {
  mimeType: string
  profiles: string[]
}

export interface StreamingRuntimeVideoPipelineProjection {
  feedbackIntervalMs: number
  nackWindowMs: number
  nackBurstCount: number
  nackMaxAgeMs: number
  nackRetryIntervalMs: number
  nackMaxRetryCount: number
  jitterBufferMinDelayMs: number
  jitterBufferMaxDelayMs: number
  jitterBufferMaxPackets: number
  idleTimeoutMs: number
  lateFrameDropThresholdMs: number
  backlogDropThresholdPackets: number
}

export interface StreamingRuntimeRecoveryProjection {
  firstFrameGraceMs: number
  keyframeRequestStallMs: number
  keyframeLossBurstThreshold: number
  decoderResetAfterKeyframeWaitMs: number
  decoderResetRequestCooldownMs: number
  reconnectStallMs: number
  stallRecoveryCooldownMs: number
}

export interface StreamingDisplayOptionsValue {
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

export type StreamingRuntimeMode = 'webrtc-direct' | 'rust-owned'
export type StreamingRuntimeOwner = 'browser' | 'sidecar'
export type StreamingBweMode = 'fixed-remb' | 'observed-remb' | 'hybrid' | 'twcc-gcc'

export interface StreamingRuntimeProjection {
  mode: StreamingRuntimeMode
  transport: StreamingRuntimeOwner
  decode: StreamingRuntimeOwner
  render: StreamingRuntimeOwner
  input: StreamingRuntimeOwner
  microphone: StreamingRuntimeOwner
  targetVideoWidth: number
  targetVideoHeight: number
  microphoneStartWithSession: boolean
  turnServer?: StreamingTurnServerConfig | null
  codec?: StreamingRuntimeCodecPreference | null
  maxVideoBitrateKbps?: number | null
  maxAudioBitrateKbps?: number | null
  forceMonoAudio: boolean
  preferIpv6: boolean
  bweMode: StreamingBweMode
  forcedRembKbps?: number | null
  adaptiveRembEnabled: boolean
  rembFloorKbps: number
  rembCeilingKbps: number
  rembRampUpStepKbps: number
  rembRampDownFactor: number
  videoPipeline: StreamingRuntimeVideoPipelineProjection
  recovery: StreamingRuntimeRecoveryProjection
  pollingRateHz: number
  vibration: boolean
  vibrationStrength: 'realistic' | 'enhanced' | 'full'
}

export interface StreamingRenderProjection {
  enableAudioControl: boolean
  videoFormat?: string | null
  displayOptions: StreamingDisplayOptionsValue
}

export type StreamingTurnSource = 'none' | 'custom' | 'fallback'

export interface StreamingSessionRegionProjection {
  name: string
  shortName?: string | null
  displayName?: string | null
  continent?: string | null
}

export interface StreamingSessionMetadataProjection {
  serverBaseUrl: string
  region?: StreamingSessionRegionProjection | null
  turnSource: StreamingTurnSource
}

export interface StreamingSessionCapabilitiesProjection {
  supportedInputs: string[]
  titleSupportsMkb: boolean
  titleSupportsTouch: boolean
  titleSupportsNativeTouch: boolean
  inputConfigResolved: boolean
  inputConfigSupportsMkb: boolean
  inputConfigSupportsTouch: boolean
  inputConfigSupportsNativeTouch: boolean
  effectiveCapabilitySource: string
  effectiveTitleSupportsMkb: boolean
  effectiveTitleSupportsTouch: boolean
  effectiveTitleSupportsNativeTouch: boolean
  runtimeSupportsNativeMkb: boolean
  runtimeSupportsTouchSurface: boolean
  remotePlayConfigurationResolved: boolean
  remotePlayRemoteManagementEnabled: boolean
  remotePlayConsoleStreamingEnabled: boolean
  effectiveRemotePlayCapabilitySource: string
  effectiveRemotePlayAllowsStreaming: boolean
  remotePlayConsoleAddrsCount: number
  inputMode: string
  touchEnabled: boolean
  microphoneStartWithSession: boolean
}

export interface StreamingSessionSnapshot {
  id: string
  targetId: string
  path: string
  targetType: StreamingTargetType
  streamState?: StreamingStreamState
  playerState: StreamingPlayerState
  queue?: StreamingQueueSnapshot
  errorDetails?: StreamingErrorDetails
}

export interface StreamingSessionExecutionSnapshot {
  session: StreamingSessionSnapshot
  runtime: StreamingRuntimeProjection
  render: StreamingRenderProjection
  metadata: StreamingSessionMetadataProjection
  capabilities: StreamingSessionCapabilitiesProjection
}

export type StreamingSessionPhase
  = | 'creating'
    | 'waitingSessionReady'
    | 'runtimeStarting'
    | 'sessionReady'
    | 'recovering'
    | 'closing'
    | 'closed'
    | 'failed'

export interface StreamingSessionProgressSnapshot {
  sessionId: string
  phase: StreamingSessionPhase
  statusTextKey: string
  queueSeconds?: number
  queue?: StreamingQueueDetails
  errorCode?: string
  errorMessage?: string
  error?: StreamingSessionError
}

export interface StreamingStartSessionParams {
  targetType: StreamingTargetType
  targetId: string
  attemptId: string
}

export interface StreamingStartSessionResult {
  attemptId: string
  execution: StreamingSessionExecutionSnapshot
  progress: StreamingSessionProgressSnapshot
}

export interface StreamingGetSessionProgressParams {
  sessionId: string
}

export interface StreamingCloseSessionParams {
  sessionId: string
}

export interface StreamingExchangeOfferParams {
  sessionId: string
  sdp: string
  channel?: 'media' | 'chat'
  restart: boolean
}

export interface StreamingExchangeOfferResult {
  answer: StreamingAnswerPayload
}

export interface StreamingSubmitIceParams {
  sessionId: string
  candidate: StreamingIceCandidate[]
  restart: boolean
}

export interface StreamingSubmitIceResult {
  accepted: boolean
}

export interface StreamingPollIceParams {
  sessionId: string
  restart: boolean
}

export interface StreamingPollIceResult {
  candidates: StreamingIceCandidate[]
}

export interface StreamingListActiveSessionsParams {
  targetType?: StreamingTargetType
}

export interface StreamingListActiveSessionsResult {
  sessions: StreamingSessionSnapshot[]
}

export interface StreamingCloseSessionResult {
  closed: boolean
}

export type StreamingRuntimeFact
  = | {
    type: 'transportConnectionState'
    connectionState: 'new' | 'connecting' | 'connected' | 'disconnected' | 'failed' | 'closed'
  }
  | {
    type: 'mediaHealth'
    connectionState: 'new' | 'connecting' | 'connected' | 'disconnected' | 'failed' | 'closed'
    connectedElapsedMs: number
    inactivityElapsedMs: number
  }
  | { type: 'mediaStalled' }

export interface StreamingDecideRecoveryParams {
  sessionId: string
  fact: StreamingRuntimeFact
  isClosing: boolean
}

export interface StreamingDecideRecoveryResult {
  shouldReconnect: boolean
  reason?: 'network-lost' | 'ice-failed' | 'media-stalled'
}
