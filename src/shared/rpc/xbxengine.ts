import type {
  StreamingRenderProjection,
  StreamingRuntimeProjection,
  StreamingTargetType,
  StreamingTurnServerConfig,
} from './streaming'

export type XbxEngineReconnectReason = 'networkLost' | 'iceFailed' | 'mediaStalled'
export type XbxEngineStreamingMode = 'cloudGaming' | 'localHost' | 'cloudHost'

export interface XbxEngineSessionDto {
  sessionId: string
  targetType: StreamingTargetType
  turnServer?: StreamingTurnServerConfig | null
}

export interface XbxEngineViewportDto {
  viewportId: string
}

export interface XbxEngineDisplayOptionsDto {
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

export interface XbxEngineDisplayStateDto {
  display_options: XbxEngineDisplayOptionsDto
}

export type XbxEngineInputEventDto
  = | {
    kind: 'pointer'
    at_ms: number
    event: 'move' | 'down' | 'up' | 'wheel' | string
    pointer_type: 'mouse' | 'touch' | 'pen' | string
    x: number
    y: number
    delta_x?: number
    delta_y?: number
    button?: number
  }
  | {
    kind: 'keyboard'
    at_ms: number
    event: 'down' | 'up' | string
    code: string
    key: string
    repeat: boolean
    ctrl_key: boolean
    shift_key: boolean
    alt_key: boolean
    meta_key: boolean
  }

export interface XbxEngineStatsDto {
  resolution: string
  rtt: string
  fps: number
  build_fingerprint?: {
    git_commit_short: string
    workspace_dirty: boolean
    build_timestamp_unix_ms: string
    cargo_profile: string
    default_feedback_interval_ms: number
    effective_feedback_interval_ms: number
    controlled_twcc_registry: boolean
  } | null
  runtime_summary?: string
  primary_issue_chain?: string
  latest_decision_summary?: string
  session_phase?: string
  transport_policy_profile?: string
  recovery_policy_profile?: string
  recovery_diagnosis?: string
  recovery_coupling_mode?: string
  recovery_coupling_summary?: string
  direct_gaming_bitrate_band?: string
  video_health?: string
  stall_kind?: string
  inbound_video_fps?: number
  decode_fps?: number
  present_fps?: number
  pl: string
  fl: string
  jit: string
  br: string
  decode: string
  transport_path?: string
  transport_state?: string
  video_rtt_source?: string
  video_remb_bps?: number
  inbound_bitrate_kbps?: number
  inbound_video_bitrate_kbps?: number
  inbound_audio_bitrate_kbps?: number
  actual_video_bitrate_source?: string
  video_bwe_mode?: string
  video_bwe_reason?: string
  video_target_remb_kbps?: number
  video_observed_remb_kbps?: number
  video_actual_bitrate_kbps?: number
  video_twcc_receive_bitrate_kbps?: number
  video_twcc_loss_ratio?: number
  video_twcc_delivery_ratio?: number
  video_twcc_feedback_interval_ms?: number
  twcc_observation_state?: string
  inbound_bytes_total?: number
  inbound_video_bytes_total?: number
  inbound_audio_bytes_total?: number
  inbound_video_packet_count_total?: number
  latest_video_track_status?: {
    state: string
    video_width?: number | null
    video_height?: number | null
    mime_type?: string | null
    transport_state: XbxEngineTransportState
    video_bytes_total: number
    video_packet_count_total: number
    audio_bytes_total: number
    observed_at_ms: number
  } | null
  video_decoder_reset_count?: number
  video_decoder_stalled?: boolean
  video_decoder_hardware_failure_streak?: number
  latest_video_decoder_hardware_failure_time_ms?: number
  latest_video_decoder_hardware_failure_status?: number
  video_renderer_stalled?: boolean
  packet_age_ms?: number
  decode_age_ms?: number
  present_age_ms?: number
  packet_to_decode_ms?: number
  decode_to_present_ms?: number
  packet_to_present_ms?: number
  video_decode_input_drop_count_total?: number
  video_decode_output_drop_count_total?: number
  video_pacer_submit_count_total?: number
  video_pacer_drop_count_total?: number
  video_renderer_submit_count_total?: number
  video_renderer_drop_count_total?: number
  video_present_drop_count_total?: number
  video_present_overwrite_count_total?: number
  video_present_submit_count_total?: number
  video_present_descriptor_upload_mode?: string
  video_present_descriptor_metal_import_count_total?: number
  video_present_descriptor_cpu_upload_count_total?: number
  recovery_keyframe_request_count?: number
  recovery_decoder_reset_count?: number
  recovery_reconnect_count?: number
  last_recovery_action?: string
  last_recovery_action_at_ms?: number
  last_recovery_reason?: string
  latest_video_packet_gap?: {
    observation_id: number
    expected_sequence: number
    received_sequence: number
    missing_count: number
    source: string
    frame_rtp_timestamp?: number | null
    frame_packet_count?: number | null
    frame_missing_count?: number | null
    frame_is_keyframe?: boolean | null
    frame_importance?: string | null
    observed_at_ms: number
  }
  latest_video_frame_drop?: {
    observation_id: number
    reason: string
    observed_at_ms: number
    width: number
    height: number
    is_keyframe: boolean
    queue_depth: number
  }
  latest_video_nack_observation?: {
    observation_id: number
    action: string
    source: string
    first_sequence: number
    last_sequence: number
    packet_count: number
    retry_count: number
    frame_rtp_timestamp?: number | null
    frame_is_keyframe?: boolean | null
    frame_importance?: string | null
    deadline_at_ms?: number | null
    observed_at_ms: number
  }
  latest_video_escalation_observation?: {
    observation_id: number
    reason: string
    action: string
    observed_at_ms: number
  }
  latest_video_bwe_observation?: {
    observation_id: number
    mode: string
    decision_reason: string
    target_remb_kbps: number
    observed_remb_kbps?: number | null
    actual_video_bitrate_kbps: number
    loss_ratio: number
    rtt_ms?: number | null
    transport_path?: string | null
    twcc_feedback_interval_ms?: number | null
    twcc_observed_packet_count?: number | null
    twcc_covered_sequence_span?: number | null
    twcc_receive_bitrate_kbps?: number | null
    twcc_delivery_ratio?: number | null
    twcc_loss_ratio?: number | null
    observed_at_ms: number
  }
  latest_video_twcc_observation?: {
    observation_id: number
    source: string
    feedback_packet_count: number
    covered_sequence_start: number
    covered_sequence_end: number
    covered_sequence_span: number
    observed_packet_count: number
    observed_byte_count: number
    coverage_ratio?: number | null
    ledger_hit_ratio?: number | null
    feedback_interval_ms?: number | null
    arrival_span_ms?: number | null
    receive_bitrate_kbps?: number | null
    quality: string
    delivery_ratio: number
    packet_loss_ratio: number
    observed_at_ms: number
  }
  latest_rtc_builder_observation?: {
    observation_id: number
    controlled_twcc_registry: boolean
    feedback_interval_ms: number
    registered_header_extensions: string[]
    registered_rtcp_feedback: string[]
    observed_at_ms: number
  }
  latest_twcc_remote_stream_observation?: {
    observation_id: number
    ssrc: number
    mime_type: string
    twcc_ext_id?: number | null
    header_extensions: string[]
    rtcp_feedback: string[]
    observed_at_ms: number
  }
  latest_remote_answer_observation?: {
    observation_id: number
    video_payload_order: number[]
    selected_video_payload_type?: number | null
    selected_video_mime_type?: string | null
    selected_video_profile_level_id?: string | null
    accepted_video_rtcp_feedback: string[]
    accepted_audio_rtcp_feedback: string[]
    accepted_video_header_extensions: string[]
    accepted_audio_header_extensions: string[]
    observed_at_ms: number
  }
  latest_twcc_extension_observation?: {
    observation_id: number
    state: string
    ssrc: number
    sequence_number: number
    expected_ext_id: number
    packet_seen_count: number
    missing_count: number
    observed_at_ms: number
  }
  latest_data_channel_message_catalog_observation?: {
    observation_id: number
    direction: string
    channel: string
    kind_type?: string | null
    kind_message?: string | null
    target?: string | null
    keys: string[]
    payload_len: number
    observed_at_ms: number
  }
}

export type XbxEngineRuntimePhase
  = | 'binding'
    | 'exchangingOffer'
    | 'gatheringIce'
    | 'exchangingIce'
    | 'connecting'
    | 'reconnecting'

export type XbxEngineTransportState
  = | 'new'
    | 'connecting'
    | 'connected'
    | 'disconnected'
    | 'failed'
    | 'closed'

export type XbxEngineRuntimeEventDto
  = | { type: 'runtime.phaseChanged', phase: XbxEngineRuntimePhase }
    | { type: 'transport.connectionState', state: XbxEngineTransportState }
    | { type: 'chat.stateChanged', capturing: boolean, paused: boolean }
    | { type: 'media.videoReady', width: number, height: number }
    | {
      type: 'media.videoTrackStatusChanged'
      status: {
        state: string
        video_width?: number | null
        video_height?: number | null
        mime_type?: string | null
        transport_state: XbxEngineTransportState
        video_bytes_total: number
        video_packet_count_total: number
        audio_bytes_total: number
        observed_at_ms: number
      }
    }
    | { type: 'media.surfaceReady', surfaceId: string }
    | {
      type: 'stats.videoFrameProcessed'
      firstFramePacketArrivalTimeMs: number
      frameDecodedTimeMs: number
      frameRenderedTimeMs: number
    }
    | { type: 'error', code: string, message: string }

export interface XbxEngineStartRuntimeParams {
  session: XbxEngineSessionDto
  viewport: XbxEngineViewportDto
  mode?: XbxEngineStreamingMode | null
  runtime: StreamingRuntimeProjection
  render: StreamingRenderProjection
  audioVolume: number
}

export interface XbxEngineAttachViewportParams {
  viewportId: string
}

export interface XbxEngineApplyDisplayStateParams {
  state: XbxEngineDisplayStateDto
}

export interface XbxEnginePressControllerButtonParams {
  button: string
  durationMs: number
}

export interface XbxEngineKeyboardPointerEnabledParams {
  enabled: boolean
}

export interface XbxEnginePushInputParams {
  event: XbxEngineInputEventDto
}

export interface XbxEngineSetAudioVolumeParams {
  value: number
}

export interface XbxEngineRequestReconnectParams {
  reason: XbxEngineReconnectReason
}

export interface XbxEngineStopRuntimeParams {
  reason?: string
}

export interface XbxEngineAckResult {
  accepted: boolean
}
