import type { StreamStats } from '../../player'
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
  let microphoneState = {
    capturing: false,
    paused: false,
  }

  function emit(event: RuntimeEvent): void {
    for (const listener of listeners) {
      listener(event)
    }
  }

  const unsubscribe = events.on('streaming.xbxEngineRuntimeEvent', (event) => {
    if (event.type === 'runtime.phaseChanged') {
      emit({ type: 'phaseChanged', phase: event.phase })
      return
    }
    if (event.type === 'transport.connectionState') {
      emit({
        type: 'connectionStateChanged',
        state: event.state as RTCPeerConnectionState,
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
      await rpc.xbxEngine.attachViewport({
        viewportId: viewportElementId,
      })
      await rpc.xbxEngine.startRuntime({
        session: {
          sessionId: spec.sessionId,
          targetType: spec.targetType,
          turnServer: spec.runtime.turnServer ?? null,
        },
        viewport: {
          viewportId: viewportElementId,
        },
        runtime: spec.runtime,
        render: spec.render,
        audioVolume,
      })
    },
    async stop(reason?: string) {
      unsubscribe()
      microphoneState = {
        capturing: false,
        paused: false,
      }
      console.info('[streaming][xbxengine-runtime] stopRuntime', {
        viewportElementId,
        reason: reason ?? 'unspecified',
      })
      await rpc.xbxEngine.stopRuntime({
        reason,
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
      return {
        resolution: snapshot.resolution,
        rtt: snapshot.rtt,
        fps: snapshot.fps,
        remoteProfileBaseline: snapshot.remote_profile_baseline,
        remoteProfileDynamic: snapshot.remote_profile_dynamic,
        remoteProfileEffectiveLabel: snapshot.remote_profile_effective_label,
        sessionPhase: snapshot.session_phase,
        transportStrategyProfile: snapshot.transport_strategy_profile,
        recoveryStrategyProfile: snapshot.recovery_strategy_profile,
        recoveryDiagnosis: snapshot.recovery_diagnosis,
        recoveryOwnerState: snapshot.recovery_owner_state,
        recoveryOwnerReason: snapshot.recovery_owner_reason,
        videoOwnerSource: snapshot.video_owner_source,
        videoOwnerObservedAtMs: snapshot.video_owner_observed_at_ms,
        directGamingBitrateBand: snapshot.direct_gaming_bitrate_band,
        videoHealth: snapshot.video_health,
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
        inboundVideoPacketCountTotal: snapshot.inbound_video_packet_count_total,
        videoDecoderResetCount: snapshot.video_decoder_reset_count,
        videoDecoderStalled: snapshot.video_decoder_stalled,
        videoDecoderRecoveryState: snapshot.video_decoder_recovery_state,
        videoDecoderRecoveryEvent: snapshot.video_decoder_recovery_event,
        videoDecoderRecoveryDetail: snapshot.video_decoder_recovery_detail,
        videoDecoderRecoveryStatus: snapshot.video_decoder_recovery_status,
        videoDecoderRecoveryStateChangedAtMs:
          snapshot.video_decoder_recovery_state_changed_at_ms,
        videoRendererStalled: snapshot.video_renderer_stalled,
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
        videoPresentDropCountTotal: snapshot.video_present_drop_count_total,
        videoPresentOverwriteCountTotal: snapshot.video_present_overwrite_count_total,
        videoPresentEnqueueCountTotal: snapshot.video_present_submit_count_total,
        videoPresentSubmitCountTotal: snapshot.video_present_submit_count_total,
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
      }
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}
