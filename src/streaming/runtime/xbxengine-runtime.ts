import type { StreamStats } from '../../player'
import type { RuntimeDisplayState, RuntimeEvent, RuntimePort, StreamRuntimeReconnectReason } from './runtime-contract'
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
    if (event.type === 'stats.videoFrameProcessed') {
      emit({ type: 'framePresented' })
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
    async stop() {
      unsubscribe()
      microphoneState = {
        capturing: false,
        paused: false,
      }
      await rpc.xbxEngine.stopRuntime()
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
        sessionPhase: snapshot.session_phase,
        transportPolicyProfile: snapshot.transport_policy_profile,
        recoveryPolicyProfile: snapshot.recovery_policy_profile,
        recoveryDiagnosis: snapshot.recovery_diagnosis,
        directGamingBitrateBand: snapshot.direct_gaming_bitrate_band,
        videoHealth: snapshot.video_health,
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
        transportState: snapshot.transport_state,
        videoRttSource: snapshot.video_rtt_source,
        videoRembBps: snapshot.video_remb_bps,
        inboundBitrateKbps: snapshot.inbound_bitrate_kbps,
        inboundVideoBitrateKbps: snapshot.inbound_video_bitrate_kbps,
        inboundAudioBitrateKbps: snapshot.inbound_audio_bitrate_kbps,
        inboundBytesTotal: snapshot.inbound_bytes_total,
        inboundVideoBytesTotal: snapshot.inbound_video_bytes_total,
        inboundAudioBytesTotal: snapshot.inbound_audio_bytes_total,
        inboundVideoPacketCountTotal: snapshot.inbound_video_packet_count_total,
        videoDecoderResetCount: snapshot.video_decoder_reset_count,
        videoDecoderStalled: snapshot.video_decoder_stalled,
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
        videoPresentSubmitCountTotal: snapshot.video_present_submit_count_total,
        videoPresentDescriptorUploadMode: snapshot.video_present_descriptor_upload_mode,
        videoPresentDescriptorMetalImportCountTotal:
          snapshot.video_present_descriptor_metal_import_count_total,
        videoPresentDescriptorCpuUploadCountTotal:
          snapshot.video_present_descriptor_cpu_upload_count_total,
        recoveryKeyframeRequestCount: snapshot.recovery_keyframe_request_count,
        recoveryDecoderResetCount: snapshot.recovery_decoder_reset_count,
        recoveryReconnectCount: snapshot.recovery_reconnect_count,
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
