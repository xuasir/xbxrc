export type VideoFit = 'Contain' | 'Stretch' | 'Zoom' | string

export type VideoFrameTrackingSource = 'videoFrameCallback' | 'timeupdate'

export type VideoFrameSourceFpsUnavailableReason
  = | 'mediaTimeMissing'
    | 'noPriorMediaTime'
    | 'mediaTimeDeltaTooSmall'
    | 'mediaTimeDeltaTooLarge'
    | 'sourceFpsOutOfRange'

export interface PresentedVideoFrameMetadata {
  callbackIntervalMs?: number
  presentedFramesDelta?: number
  mediaTimeDeltaSec?: number
  expectedDisplayLeadMs?: number
  rawSourceFpsEstimate?: number
  sourceFpsEstimate?: number
  sourceFrameIntervalMs?: number
  sourceFpsUnavailableReason?: VideoFrameSourceFpsUnavailableReason
  trackingSource?: VideoFrameTrackingSource
  droppedLike: boolean
}

export interface RendererPresentTarget {
  outputWidth: number
  outputHeight: number
  viewportWidthCss: number
  viewportHeightCss: number
  displayWidthCss: number
  displayHeightCss: number
  devicePixelRatio: number
  fullscreen: boolean
  refreshRateHz?: number
  sourceWidth: number
  sourceHeight: number
}

/** 仅描述 renderer attach 所需的最小合同；策略字段留在 RendererRuntimeConfig。 */
export interface RendererAttachSpec {
  kind: 'video' | 'webgl2' | 'webgl2_sr'
  targetFps: number
  format: VideoFit
  brightness: number
  contrast: number
  saturation: number
  processing?: 'usm' | 'cas'
  processingMode?: 'quality' | 'performance'
  shaderPreset?: 'clarityL0' | 'clarityL1' | 'clarityL2' | 'clarityL3'
  sharpenStrength?: number
  presentTarget?: RendererPresentTarget
  sr?: {
    outputWidth: number
    outputHeight: number
    rcasStops: number
  }
}

/** 将 attach 合同合并进 runtime config，供 WebGL / SR renderer 读取尺寸与管线字段。 */
export function mergeRendererConfigWithAttachSpec(
  config: RendererRuntimeConfig,
  attach: RendererAttachSpec,
): RendererRuntimeConfig {
  const pipelineType: RendererRuntimeConfig['pipelineType'] = attach.kind === 'video' ? 'video' : 'webgl2'
  const mode: RendererRuntimeConfig['mode'] = attach.kind === 'video' ? 'native' : 'webgl2'
  return {
    ...config,
    targetFps: attach.targetFps,
    format: attach.format,
    brightness: attach.brightness,
    contrast: attach.contrast,
    saturation: attach.saturation,
    processing: attach.processing ?? config.processing,
    processingMode: attach.processingMode ?? config.processingMode,
    shaderPreset: attach.shaderPreset ?? config.shaderPreset,
    sharpenStrength: attach.sharpenStrength ?? config.sharpenStrength,
    pipelineType,
    mode,
    superResolutionEnabled: attach.kind === 'webgl2_sr',
    superResolutionOutputWidth: attach.sr?.outputWidth,
    superResolutionOutputHeight: attach.sr?.outputHeight,
    superResolutionRcasStops: attach.sr?.rcasStops,
    renderOutputWidth: attach.presentTarget?.outputWidth,
    renderOutputHeight: attach.presentTarget?.outputHeight,
    renderViewportWidth: attach.presentTarget?.viewportWidthCss,
    renderViewportHeight: attach.presentTarget?.viewportHeightCss,
    renderDisplayWidth: attach.presentTarget?.displayWidthCss,
    renderDisplayHeight: attach.presentTarget?.displayHeightCss,
    renderDevicePixelRatio: attach.presentTarget?.devicePixelRatio,
    renderDisplayFullscreen: attach.presentTarget?.fullscreen,
    renderDisplayRefreshHz: attach.presentTarget?.refreshRateHz,
    renderSourceWidth: attach.presentTarget?.sourceWidth,
    renderSourceHeight: attach.presentTarget?.sourceHeight,
  }
}

export function deriveRendererAttachSpec(config: RendererRuntimeConfig): RendererAttachSpec {
  let base: 'video' | 'webgl2' = 'video'
  if (!config.enabled) {
    base = 'video'
  }
  else if (config.pipelineType === 'video') {
    base = 'video'
  }
  else if (config.pipelineType === 'webgl2') {
    base = 'webgl2'
  }
  else {
    base = config.mode === 'webgl2' ? 'webgl2' : 'video'
  }
  const kind: RendererAttachSpec['kind'] = base === 'webgl2'
    && config.superResolutionEnabled === true
    && config.superResolutionInactiveAfterFailure !== true
    ? 'webgl2_sr'
    : base
  const sr = kind === 'webgl2_sr'
    && config.superResolutionOutputWidth !== undefined
    && config.superResolutionOutputHeight !== undefined
    ? {
        outputWidth: config.superResolutionOutputWidth,
        outputHeight: config.superResolutionOutputHeight,
        rcasStops: config.superResolutionRcasStops ?? 0.88,
      }
    : undefined
  return {
    kind,
    targetFps: config.targetFps,
    format: config.format,
    brightness: config.brightness,
    contrast: config.contrast,
    saturation: config.saturation,
    processing: config.processing,
    processingMode: config.processingMode,
    shaderPreset: config.shaderPreset,
    sharpenStrength: config.sharpenStrength,
    presentTarget: config.renderOutputWidth !== undefined
      && config.renderOutputHeight !== undefined
      && config.renderViewportWidth !== undefined
      && config.renderViewportHeight !== undefined
      && config.renderDisplayWidth !== undefined
      && config.renderDisplayHeight !== undefined
      && config.renderDevicePixelRatio !== undefined
      && config.renderDisplayFullscreen !== undefined
      && config.renderSourceWidth !== undefined
      && config.renderSourceHeight !== undefined
      ? {
          outputWidth: config.renderOutputWidth,
          outputHeight: config.renderOutputHeight,
          viewportWidthCss: config.renderViewportWidth,
          viewportHeightCss: config.renderViewportHeight,
          displayWidthCss: config.renderDisplayWidth,
          displayHeightCss: config.renderDisplayHeight,
          devicePixelRatio: config.renderDevicePixelRatio,
          fullscreen: config.renderDisplayFullscreen,
          refreshRateHz: config.renderDisplayRefreshHz,
          sourceWidth: config.renderSourceWidth,
          sourceHeight: config.renderSourceHeight,
        }
      : undefined,
    sr,
  }
}

export interface AudioRuntimeConfig {
  volume: number
  enableAudioControl: boolean
}

export interface RendererRuntimeConfig {
  enabled: boolean
  sharpness: number
  sharpenStrength?: number
  /** clarity 预设仅映射本地锐化强度与算法，不代表 FSR 超分链路。 */
  shaderPreset?: 'clarityL0' | 'clarityL1' | 'clarityL2' | 'clarityL3'
  pipelineType: 'video' | 'webgl2' | 'auto'
  /** 当前仅支持 USM/CAS 锐化后处理。 */
  processing: 'usm' | 'cas'
  processingMode: 'quality' | 'performance'
  brightness: number
  contrast: number
  saturation: number
  targetFps: number
  mode: 'native' | 'webgl2'
  format: VideoFit
  /** 用户意图：开启后优先走独立 SR renderer，不因 display degrade 动态关闭。 */
  superResolutionEnabled?: boolean
  superResolutionAlgorithm?: 'fsr1'
  superResolutionOutputTier?: '720p' | '1080p' | '1440p' | '2160p'
  superResolutionConfiguredTargetTier?: string
  superResolutionOutputWidth?: number
  superResolutionOutputHeight?: number
  /**
   * 直接传给 FSR1 `FsrRcasCon` 的 sharpness stops。
   * 值越大越柔和；0.88 为当前串流实验默认档。
   */
  superResolutionRcasStops?: number
  /** SR 技术性失败回退到标准 webgl2 时使用的锐化算法。 */
  superResolutionFallbackProcessing?: 'usm' | 'cas'
  /** 本会话内 SR attach 失败后置位，阻止再次选用 SR renderer。 */
  superResolutionInactiveAfterFailure?: boolean
  renderOutputWidth?: number
  renderOutputHeight?: number
  renderViewportWidth?: number
  renderViewportHeight?: number
  renderDisplayWidth?: number
  renderDisplayHeight?: number
  renderDevicePixelRatio?: number
  renderDisplayFullscreen?: boolean
  renderDisplayRefreshHz?: number
  renderSourceWidth?: number
  renderSourceHeight?: number
  /**
   * 仅客户端注入：SR 在 attach 成功后绘制期仍失败（如持续 GL 错误）时回调，
   * 由 PlaybackService 切回标准 webgl2。不向 Rust 序列化。
   */
  superResolutionRuntimeDegradeNotifier?: (reason: string) => void
}

export interface StreamStats {
  resolution: string
  rtt: string
  fps: number
  streamLifecyclePhase?: string
  presentationMilestone?: string
  connectedMilestoneElapsedMs?: number
  mediaReadyMilestoneElapsedMs?: number
  presentationFailedStage?: string
  remoteProfileBaseline?: string
  remoteProfileDynamic?: string
  remoteProfileEffectiveLabel?: string
  sessionPhase?: string
  transportStrategyProfile?: string
  recoveryStrategyProfile?: string
  /** 后端聚合展示；优先看 recoveryOwnerReason、primaryIssueChain 与 RFC 三字段。 */
  diagnosis?: string
  recoveryRfcFaultDomain?: string
  recoveryRfcStage?: string
  recoveryRfcCeiling?: string
  recoveryOwnerState?: string
  recoveryOwnerReason?: string
  videoOwnerSource?: string
  videoOwnerObservedAtMs?: number
  remoteProfileBitrateBand?: string
  videoHealth?: string
  chainHealth?: string
  presentationHealth?: string
  /** 与 videoHealth 并列的主诊断链（如 steady:healthy / display:supplyStarved） */
  primaryIssueChain?: string
  latestDecisionSummary?: string
  stallKind?: string
  inboundVideoFps?: number
  decodeFps?: number
  presentFps?: number
  pl: string
  fl: string
  jit: string
  br: string
  decode: string
  transportPath?: string
  transportCandidatePair?: string
  transportProtocol?: string
  transportAddressFamily?: 'ipv4' | 'ipv6' | 'mixed' | 'unknown'
  transportState?: string
  icePolicyMode?: 'passthrough' | 'policy'
  icePolicyDigest?: string
  videoRttSource?: string
  videoRembBps?: number
  inboundBitrateKbps?: number
  inboundVideoBitrateKbps?: number
  inboundAudioBitrateKbps?: number
  latestAudioPlayoutTimeMs?: number
  audioPlayoutLatencyMs?: number
  audioVideoPlayoutDeltaMs?: number
  actualVideoBitrateSource?: string
  videoBweMode?: string
  videoBweReason?: string
  videoTargetRembKbps?: number
  videoObservedRembKbps?: number
  bandwidthState?: 'stable' | 'warning' | 'congested' | 'recovering'
  bandwidthAction?: 'none' | 'observe' | 'downshift' | 'keyframeRequest' | 'decoderReset' | 'reconnect'
  recoveryEpochId?: string
  lastRecoveryActionLevel?: 'L0' | 'L1' | 'L2' | 'L3'
  lastRecoveryActionResult?: 'planned' | 'executed' | 'suppressed' | 'notSupported' | 'failed'
  recoverySuppressedBy?: 'factWindow' | 'reasonWindow' | 'cooldown' | 'budget' | 'channelUnhealthy' | 'unknown'
  recoveryBudgetRemaining?: string
  controlChannelState?: string
  lastControlChannelError?: string
  keyframeRequestSuccessRate?: number
  controlChannelOpenRatio?: number
  controlChannelBufferedTrend?: 'rising' | 'stable' | 'falling'
  controlChannelSendFailBurst?: number
  lastRecoveryActionEffect?: 'improved' | 'neutral' | 'degraded' | 'unknown'
  lastRecoveryActionEffectScore?: number
  lastRecoveryActionEffectReason?: string
  networkConfidence?: 'high' | 'low'
  decodeConfidence?: 'high' | 'low'
  recoveryCause?: 'networkCongestion' | 'decodeBackpressure' | 'renderStarvation' | 'controlChannelUnhealthy' | 'unknown'
  qualityLadderLevel?: 'L0' | 'L1' | 'L2'
  decisionDigest?: string
  firstFrameStage?: 'idle' | 'connecting' | 'firstDecoded' | 'firstPresented'
  firstFrameStageChangedAtMs?: number
  firstDecodedAtMs?: number
  firstPresentedAtMs?: number
  firstFrameGuardTriggered?: boolean
  renderBackpressure?: boolean
  renderDroppedFrames?: number
  renderCallbackGapCount?: number
  renderFrameCallbackIntervalMs?: number
  renderCallbackCountLastSample?: number
  renderCallbackGapCountLastSample?: number
  renderFrameTrackingSource?: 'videoFrameCallback' | 'timeupdate'
  renderPresentedFramesDelta?: number
  renderPresentedFramesJumpCount?: number
  renderPresentedFramesAdvancedLastSample?: number
  renderPresentedFramesJumpCountLastSample?: number
  renderFrameMediaTimeDeltaSec?: number
  renderFrameExpectedDisplayLeadMs?: number
  renderFrameRawSourceFpsEstimate?: number
  renderFrameSourceFpsEstimate?: number
  renderFrameSourceFrameIntervalMs?: number
  renderFrameSourceFpsUnavailableReason?: VideoFrameSourceFpsUnavailableReason
  renderDroppedLikeStreak?: number
  renderCause?: 'decodeBackpressure' | 'renderStarvation' | 'renderStable'
  displayDegradeLevel?: 'displayL0' | 'displayL1' | 'displayL2'
  renderDecisionDigest?: string
  senderPolicyCause?: 'networkCongestion' | 'decodeBackpressure' | 'controlChannelUnhealthy' | 'none'
  videoActualBitrateKbps?: number
  videoTwccReceiveBitrateKbps?: number
  videoTwccLossRatio?: number
  videoTwccDeliveryRatio?: number
  videoTwccFeedbackIntervalMs?: number
  twccObservationState?: string
  inboundBytesTotal?: number
  inboundVideoBytesTotal?: number
  inboundAudioBytesTotal?: number
  inboundVideoPacketCountTotal?: number
  latestVideoPacketArrivalRtpTimestamp?: number
  videoTrackStatus?: {
    state: string
    videoWidth?: number | null
    videoHeight?: number | null
    mimeType?: string | null
    transportState?: string
    videoBytesTotal: number
    videoPacketCountTotal: number
    audioBytesTotal: number
    observedAtMs: number
  }
  videoDecoderResetCount?: number
  videoDecoderStalled?: boolean
  videoDecoderRecoveryState?: string
  videoDecoderRecoveryEvent?: string
  videoDecoderRecoveryDetail?: string
  videoDecoderRecoveryStatus?: number
  videoDecoderRecoveryStateChangedAtMs?: number
  latestVideoDecodeOkRtpTimestamp?: number
  videoRendererStalled?: boolean
  videoRendererStallBlocksPresentation?: boolean
  packetAgeMs?: number
  decodeAgeMs?: number
  presentAgeMs?: number
  packetToDecodeMs?: number
  decodeToPresentMs?: number
  packetToPresentMs?: number
  videoDecodeInputDropCountTotal?: number
  videoDecodeOutputDropCountTotal?: number
  videoPacerSubmitCountTotal?: number
  videoPacerDropCountTotal?: number
  videoRendererSubmitCountTotal?: number
  videoRendererDropCountTotal?: number
  hostMailboxDropCountTotal?: number
  hostMailboxOverwriteCountTotal?: number
  hostMailboxEnqueueCountTotal?: number
  videoPresentDescriptorUploadMode?: string
  videoPresentDescriptorMetalImportCountTotal?: number
  videoPresentDescriptorCpuUploadCountTotal?: number
  hostMailboxSubmitEpoch?: number
  hostDisplayTickEpoch?: number
  hostFramePresentEpoch?: number
  hostMailboxLatestSubmitAtMs?: number
  latestVideoHostSubmitRtpTimestamp?: number
  submitAgeMs?: number
  displayAgeMs?: number
  hostViewGeneration?: number
  latestHostViewCreatedAtMs?: number
  lastDisplayedFrameSeq?: number
  lastDisplayedFrameRtpTimestamp?: number
  lastDisplayedAtMs?: number
  recoveryKeyframeRequestCount?: number
  recoveryDecoderResetCount?: number
  recoveryReconnectCount?: number
  recoveryHardFallbackTimerMs?: number
  recoveryHardFallbackTriggerReason?: string
  recoveryHardFallbackTimerResetReason?: string
  lastRecoveryAction?: string
  lastRecoveryActionAtMs?: number
  lastRecoveryReason?: string
  renderAdaptiveProfileDigest?: string
  renderHysteresisState?: 'steady' | 'holdDown' | 'holdUp'
  renderUpshiftBlockedReason?: string
  renderSuperResolutionEnabled?: boolean
  renderSuperResolutionActive?: boolean
  renderSuperResolutionAlgorithm?: string
  renderSuperResolutionConfiguredTarget?: string
  renderSuperResolutionOutputTarget?: string
  renderSuperResolutionRcasStops?: number
  renderSuperResolutionRcasBaseStops?: number
  renderSuperResolutionFallbackReason?: string | null
  renderSharpenMode?: string
  renderPipelineType?: 'video' | 'webgl2' | 'webgl2_sr'
  renderPolicySource?: 'auto' | 'userOverride' | 'capabilityFallback' | 'srFallback'
  renderProcessing?: 'usm' | 'cas'
  renderProcessingMode?: 'quality' | 'performance'
  renderShaderPath?: 'usm' | 'cas' | 'none'
  renderFpsBudget?: number
  rendererCapabilityReason?: string
  renderDisplayWidth?: number
  renderDisplayHeight?: number
  renderDisplayFullscreen?: boolean
  renderDisplayRefreshHz?: number
  renderPresentTargetWidth?: number
  renderPresentTargetHeight?: number
  renderViewportWidth?: number
  renderViewportHeight?: number
  renderSourceWidth?: number
  renderSourceHeight?: number
  frontEndProfileBaseline?: 'homeLan' | 'homeRelay' | 'cloud'
  frontEndProfileDynamic?: 'startup' | 'steady' | 'highRtt' | 'decoderConstrained' | 'displayConstrained'
  frontEndContentFpsClass?: 'content30' | 'content60' | 'contentUnknown'
  frontEndExpectedContentFps?: number
  frontEndPolicyPreset?: string
  frontEndPolicyInputReason?: 'healthy' | 'networkLimited' | 'deliveryLimited'
  frontEndWarmupUntilMs?: number
  frontEndUpshiftBlockedReason?: string
}
