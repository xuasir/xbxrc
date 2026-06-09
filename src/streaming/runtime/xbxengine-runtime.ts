import type { StreamStats } from '../../player'
import type { XbxEngineStatsDto } from '../../shared/rpc/xbxengine'
import type {
  RuntimeDisplayState,
  RuntimeEvent,
  RuntimePort,
  StreamRuntimeReconnectReason,
} from './runtime-contract'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'

function toRuntimeReconnectReason(reason: StreamRuntimeReconnectReason) {
  if (reason === 'ice-failed') {
    return 'iceFailed' as const
  }
  if (reason === 'media-stalled') {
    return 'mediaStalled' as const
  }
  return 'networkLost' as const
}

/**
 * Rust runtime 通过 RPC 适配到同一套 runtime/UI 协议。
 */
export function createXbxEngineRuntime(options: {
  playerElementId: string
  initialAudioVolume: number
}): RuntimePort {
  const listeners = new Set<(event: RuntimeEvent) => void>()
  const viewportElementId = options.playerElementId
  let audioVolume = options.initialAudioVolume
  let transportState: RTCPeerConnectionState = 'new'
  let microphoneState = {
    capturing: false,
    paused: false,
  }
  let pendingLegacyMediaReady = false

  function emit(event: RuntimeEvent): void {
    for (const listener of listeners) {
      listener(event)
    }
  }

  function emitLegacyMediaReadyMilestone(now: number): void {
    pendingLegacyMediaReady = false
    emit({
      type: 'presentationMilestoneChanged',
      milestone: 'connected',
      connectedAtMs: now,
      mediaReadyAtMs: null,
      stage: 'connected',
    })
    emit({
      type: 'presentationMilestoneChanged',
      milestone: 'mediaReady',
      connectedAtMs: now,
      mediaReadyAtMs: now,
      stage: 'mediaReady',
    })
  }

  function recordLaunchTraceEvent(
    event: string,
    payload: Record<string, unknown>,
    sessionId?: string,
  ): void {
    void rpc.runtimeTrace.recordEvent({
      event,
      sessionId,
      payload,
    }).catch(() => {
      // trace 失败不能影响 runtime 启动主链
    })
  }

  const unsubscribe = events.on('streaming.xbxEngineRuntimeEvent', (event) => {
    if (event.type === 'runtime.phaseChanged') {
      emit({ type: 'phaseChanged', phase: event.phase })
      return
    }
    if (event.type === 'transport.connectionState') {
      transportState = event.state as RTCPeerConnectionState
      emit({
        type: 'connectionStateChanged',
        state: transportState,
      })
      if (transportState === 'connected' && pendingLegacyMediaReady) {
        emitLegacyMediaReadyMilestone(Date.now())
      }
      return
    }
    if (event.type === 'media.videoReady') {
      pendingLegacyMediaReady = true
      if (transportState === 'connected') {
        emitLegacyMediaReadyMilestone(Date.now())
      }
      return
    }
    if (event.type === 'presentation.milestoneChanged') {
      emit({
        type: 'presentationMilestoneChanged',
        milestone: event.milestone,
        connectedAtMs: event.connectedAtMs ?? null,
        mediaReadyAtMs: event.mediaReadyAtMs ?? null,
        stage: event.stage ?? null,
      })
      return
    }
    if (event.type === 'chat.stateChanged') {
      microphoneState = {
        capturing: event.capturing,
        paused: event.paused,
      }
      emit({ type: 'microphoneStateChanged', ...microphoneState })
      return
    }
    if (event.type === 'stats.videoFrameRendered') {
      emit({ type: 'frameReady' })
      return
    }
    if (event.type === 'error') {
      emit({
        type: 'error',
        error: new Error(`${event.code}:${event.message}`),
      })
    }
  })

  return {
    async launch(spec) {
      recordLaunchTraceEvent('runtimeAttachViewportRequested', {
        viewportId: viewportElementId,
        targetType: spec.targetType,
      }, spec.sessionId)
      await rpc.xbxEngine.attachViewport({
        viewportId: viewportElementId,
      })
      recordLaunchTraceEvent('runtimeAttachViewportCompleted', {
        viewportId: viewportElementId,
        targetType: spec.targetType,
      }, spec.sessionId)
      recordLaunchTraceEvent('runtimeStartRequested', {
        viewportId: viewportElementId,
        targetType: spec.targetType,
        turnMode: spec.runtime.turnServer === null ? 'direct' : 'fallback',
      }, spec.sessionId)
      await rpc.xbxEngine.startRuntime({
        session: {
          sessionId: spec.sessionId,
          targetType: spec.targetType,
          turnServer: spec.runtime.turnServer ?? null,
        },
        viewport: {
          viewportId: viewportElementId,
        },
        runtime: ((runtime) => {
          const { iceCandidatePolicy: _iceCandidatePolicy, ...rest } = runtime as unknown as Record<string, unknown>
          return rest as unknown as typeof spec.runtime
        })(spec.runtime),
        render: spec.render,
        iceCandidatePolicy: spec.runtime.iceCandidatePolicy ?? null,
        audioVolume,
      })
      recordLaunchTraceEvent('runtimeStartCompleted', {
        viewportId: viewportElementId,
        targetType: spec.targetType,
        turnMode: spec.runtime.turnServer === null ? 'direct' : 'fallback',
      }, spec.sessionId)
    },
    async stop(reason?: string) {
      unsubscribe()
      microphoneState = {
        capturing: false,
        paused: false,
      }
      recordLaunchTraceEvent('runtimeStopRequested', {
        viewportId: viewportElementId,
        reason: reason ?? 'unspecified',
      })
      await rpc.xbxEngine.stopRuntime({
        reason,
      })
      recordLaunchTraceEvent('runtimeStopCompleted', {
        viewportId: viewportElementId,
        reason: reason ?? 'unspecified',
      })
    },
    async requestReconnect(reason) {
      await rpc.xbxEngine.requestReconnect({
        reason: toRuntimeReconnectReason(reason),
      })
    },
    applyDisplayState(state: RuntimeDisplayState) {
      void rpc.xbxEngine.applyDisplayState({
        state: {
          video_format: state.render.videoFormat ?? null,
          display_options: {
            sharpness: state.displayOptions.sharpness,
            saturation: state.displayOptions.saturation,
            contrast: state.displayOptions.contrast,
            brightness: state.displayOptions.brightness,
          },
        },
      })
    },
    setAudioVolume(value) {
      audioVolume = value
      void rpc.xbxEngine.setAudioVolume({ value })
    },
    async setMicrophoneEnabled(enabled) {
      if (enabled) {
        await rpc.xbxEngine.startMicrophone()
        return true
      }
      await rpc.xbxEngine.stopMicrophone()
      return false
    },
    pressHome(durationMs) {
      void rpc.xbxEngine.pressControllerButton({
        button: 'home',
        durationMs,
      })
    },
    snapshotStats: async (): Promise<StreamStats> => {
      const snapshot = await rpc.xbxEngine.snapshotStats()
      const streamLifecyclePhase = resolveRuntimeLifecyclePhase(snapshot)
      return {
        resolution: snapshot.resolution,
        rtt: snapshot.rtt,
        fps: snapshot.fps,
        remoteProfileBaseline: snapshot.remote_profile_baseline,
        remoteProfileDynamic: snapshot.remote_profile_dynamic,
        remoteProfileEffectiveLabel: snapshot.remote_profile_effective_label,
        streamLifecyclePhase,
        presentationMilestone: snapshot.presentation_milestone ?? undefined,
        connectedMilestoneElapsedMs: snapshot.connected_milestone_elapsed_ms ?? undefined,
        mediaReadyMilestoneElapsedMs: snapshot.media_ready_milestone_elapsed_ms ?? undefined,
        presentationFailedStage: snapshot.presentation_failed_stage ?? undefined,
        sessionPhase: streamLifecyclePhase,
        transportStrategyProfile: snapshot.transport_strategy_profile,
        recoveryStrategyProfile: snapshot.recovery_strategy_profile,
        diagnosis: snapshot.recovery_diagnosis,
        recoveryRfcFaultDomain: snapshot.recovery_rfc_fault_domain,
        recoveryRfcStage: snapshot.recovery_rfc_stage,
        recoveryRfcCeiling: snapshot.recovery_rfc_ceiling,
        recoveryOwnerState: snapshot.recovery_owner_state,
        recoveryOwnerReason: snapshot.recovery_owner_reason,
        videoOwnerSource: snapshot.video_owner_source,
        videoOwnerObservedAtMs: snapshot.video_owner_observed_at_ms,
        remoteProfileBitrateBand: snapshot.direct_gaming_bitrate_band,
        videoHealth: snapshot.video_health,
        chainHealth: snapshot.chain_health,
        presentationHealth: snapshot.presentation_health,
        primaryIssueChain: snapshot.primary_issue_chain,
        latestDecisionSummary: snapshot.latest_decision_summary,
        stallKind: snapshot.stall_kind,
        inboundVideoFps: snapshot.inbound_video_fps,
        decodeFps: snapshot.decode_fps,
        presentFps: snapshot.present_fps,
        pl: snapshot.pl,
        fl: snapshot.fl,
        jit: snapshot.jit,
        br: snapshot.br,
        decode: snapshot.decode,
        transportPath: snapshot.transport_path,
        transportCandidatePair: snapshot.transport_candidate_pair,
        transportProtocol: snapshot.transport_protocol,
        transportAddressFamily: snapshot.transport_address_family ?? undefined,
        transportState: snapshot.transport_state,
        icePolicyMode: snapshot.ice_policy_mode === 'policy'
          ? 'policy'
          : snapshot.ice_policy_mode === 'passthrough'
            ? 'passthrough'
            : undefined,
        icePolicyDigest: snapshot.ice_policy_digest ?? undefined,
        videoRttSource: snapshot.video_rtt_source,
        videoRembBps: snapshot.video_remb_bps,
        inboundBitrateKbps: snapshot.inbound_bitrate_kbps,
        inboundVideoBitrateKbps: snapshot.inbound_video_bitrate_kbps,
        inboundAudioBitrateKbps: snapshot.inbound_audio_bitrate_kbps,
        latestAudioPlayoutTimeMs: snapshot.latest_audio_playout_time_ms,
        audioPlayoutLatencyMs: snapshot.audio_playout_latency_ms,
        audioVideoPlayoutDeltaMs: snapshot.audio_video_playout_delta_ms,
        actualVideoBitrateSource: snapshot.actual_video_bitrate_source,
        videoBweMode: snapshot.video_bwe_mode,
        videoBweReason: snapshot.video_bwe_reason,
        videoTargetRembKbps: snapshot.video_target_remb_kbps,
        videoObservedRembKbps: snapshot.video_observed_remb_kbps,
        videoActualBitrateKbps: snapshot.video_actual_bitrate_kbps,
        videoTwccReceiveBitrateKbps: snapshot.video_twcc_receive_bitrate_kbps,
        videoTwccLossRatio: snapshot.video_twcc_loss_ratio,
        videoTwccDeliveryRatio: snapshot.video_twcc_delivery_ratio,
        videoTwccFeedbackIntervalMs: snapshot.video_twcc_feedback_interval_ms,
        twccObservationState: snapshot.twcc_observation_state,
        inboundBytesTotal: snapshot.inbound_bytes_total,
        inboundVideoBytesTotal: snapshot.inbound_video_bytes_total,
        inboundAudioBytesTotal: snapshot.inbound_audio_bytes_total,
        inboundVideoFrameCountTotal: snapshot.inbound_video_frame_count_total,
        inboundVideoRtpMarkerCountTotal: snapshot.inbound_video_rtp_marker_count_total,
        inboundVideoAccessUnitCountTotal: snapshot.inbound_video_access_unit_count_total,
        inboundVideoDecodeGateEmitCountTotal:
          snapshot.inbound_video_decode_gate_emit_count_total,
        inboundVideoDecodeGateContinueCountTotal:
          snapshot.inbound_video_decode_gate_continue_count_total,
        inboundVideoPacketCountTotal: snapshot.inbound_video_packet_count_total,
        latestVideoPacketArrivalRtpTimestamp: snapshot.latest_video_packet_arrival_rtp_timestamp,
        videoDecoderResetCount: snapshot.video_decoder_reset_count,
        videoDecoderStalled: snapshot.video_decoder_stalled,
        videoDecoderRecoveryState: snapshot.video_decoder_recovery_state,
        videoDecoderRecoveryEvent: snapshot.video_decoder_recovery_event,
        videoDecoderRecoveryDetail: snapshot.video_decoder_recovery_detail,
        videoDecoderRecoveryStatus: snapshot.video_decoder_recovery_status,
        videoDecoderRecoveryStateChangedAtMs:
          snapshot.video_decoder_recovery_state_changed_at_ms,
        latestVideoDecodeOkRtpTimestamp: snapshot.latest_video_decode_ok_rtp_timestamp,
        videoRendererStalled: snapshot.video_renderer_stalled,
        videoRendererStallBlocksPresentation:
          snapshot.video_renderer_stall_blocks_presentation,
        packetAgeMs: snapshot.packet_age_ms,
        decodeAgeMs: snapshot.decode_age_ms,
        presentAgeMs: snapshot.present_age_ms,
        packetToDecodeMs: snapshot.packet_to_decode_ms,
        decodeToPresentMs: snapshot.decode_to_present_ms,
        packetToPresentMs: snapshot.packet_to_present_ms,
        videoDecodeInputDropCountTotal: snapshot.video_decode_input_drop_count_total,
        videoDecodeOutputDropCountTotal: snapshot.video_decode_output_drop_count_total,
        videoPacerSubmitCountTotal: snapshot.video_pacer_submit_count_total,
        videoPacerDropCountTotal: snapshot.video_pacer_drop_count_total,
        videoRendererSubmitCountTotal: snapshot.video_renderer_submit_count_total,
        videoRendererDropCountTotal: snapshot.video_renderer_drop_count_total,
        hostMailboxDropCountTotal: snapshot.host_mailbox_drop_count_total,
        hostMailboxOverwriteCountTotal: snapshot.host_mailbox_overwrite_count_total,
        hostMailboxEnqueueCountTotal: snapshot.host_mailbox_enqueue_count_total,
        videoPresentDescriptorUploadMode: snapshot.video_present_descriptor_upload_mode,
        videoPresentDescriptorMetalImportCountTotal:
          snapshot.video_present_descriptor_metal_import_count_total,
        videoPresentDescriptorCpuUploadCountTotal:
          snapshot.video_present_descriptor_cpu_upload_count_total,
        recoveryKeyframeRequestCount: snapshot.recovery_keyframe_request_count,
        recoveryDecoderResetCount: snapshot.recovery_decoder_reset_count,
        recoveryReconnectCount: snapshot.recovery_reconnect_count,
        recoveryHardFallbackTimerMs: snapshot.recovery_hard_fallback_timer_ms,
        recoveryHardFallbackTriggerReason: snapshot.recovery_hard_fallback_trigger_reason,
        recoveryHardFallbackTimerResetReason:
          snapshot.recovery_hard_fallback_timer_reset_reason,
        lastRecoveryAction: snapshot.last_recovery_action,
        lastRecoveryActionAtMs: snapshot.last_recovery_action_at_ms,
        lastRecoveryReason: snapshot.last_recovery_reason,
        hostMailboxSubmitEpoch: snapshot.host_mailbox_submit_epoch,
        hostDisplayTickEpoch: snapshot.host_display_tick_epoch,
        hostFramePresentEpoch: snapshot.host_frame_present_epoch,
        hostMailboxLatestSubmitAtMs: snapshot.latest_host_mailbox_submit_time_ms,
        latestVideoHostSubmitRtpTimestamp: snapshot.latest_video_host_submit_rtp_timestamp,
        submitAgeMs: snapshot.submit_age_ms,
        displayAgeMs: snapshot.display_age_ms,
        hostViewGeneration: snapshot.host_view_generation,
        latestHostViewCreatedAtMs: snapshot.latest_host_view_created_at_ms,
        lastDisplayedFrameSeq: snapshot.last_displayed_frame_seq,
        lastDisplayedFrameRtpTimestamp: snapshot.last_displayed_frame_rtp_timestamp,
        lastDisplayedAtMs: snapshot.last_displayed_at_ms,
      }
    },
    captureRenderedFrame: async () => null,
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}

function resolveRuntimeLifecyclePhase(snapshot: XbxEngineStatsDto): string | undefined {
  return snapshot.runtime_lifecycle_phase
    ?? snapshot.unified_lifecycle_phase
    ?? snapshot.session_lifecycle_phase
    ?? snapshot.stream_lifecycle_phase
    ?? snapshot.session_phase
}
