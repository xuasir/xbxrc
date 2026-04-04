use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEngineTargetTypeDto {
    Home,
    Cloud,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineTurnServerDto {
    pub url: String,
    pub username: String,
    pub credential: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineSessionDto {
    pub session_id: String,
    pub target_type: XbxEngineTargetTypeDto,
    pub turn_server: Option<XbxEngineTurnServerDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XbxEngineViewportDto {
    pub viewport_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEngineRuntimePhaseDto {
    Binding,
    ExchangingOffer,
    GatheringIce,
    ExchangingIce,
    Connecting,
    Reconnecting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEngineTransportStateDto {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XbxEngineVideoTrackStatusDto {
    pub state: String,
    pub video_width: Option<u32>,
    pub video_height: Option<u32>,
    pub mime_type: Option<String>,
    pub transport_state: XbxEngineTransportStateDto,
    pub video_bytes_total: u64,
    pub video_packet_count_total: u64,
    pub audio_bytes_total: u64,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum XbxEngineRuntimeEventDto {
    RuntimePhaseChanged {
        phase: XbxEngineRuntimePhaseDto,
    },
    TransportConnectionStateChanged {
        state: XbxEngineTransportStateDto,
    },
    ChatStateChanged {
        capturing: bool,
        paused: bool,
    },
    MediaVideoReady {
        width: u32,
        height: u32,
    },
    MediaVideoTrackStatusChanged {
        status: XbxEngineVideoTrackStatusDto,
    },
    MediaSurfaceReady {
        surface_id: String,
    },
    /// `renderer_frame_time_ms` 是 renderer 管线完成时间，不等同宿主 host present 时间。
    /// present freshness 请使用 stats 中的 `latest_video_host_present_time_ms / present_age_ms`。
    StatsVideoFrameRendered {
        first_frame_packet_arrival_time_ms: f64,
        frame_decoded_time_ms: f64,
        renderer_frame_time_ms: f64,
    },
    DiagnosticsPulse {
        window_ms: f64,
        frames_in_window: u64,
        fps: f64,
        render_idle_ms: Option<f64>,
        inbound_kbps: f64,
        inbound_video_kbps: f64,
        inbound_primary_video_kbps: f64,
        inbound_audio_kbps: f64,
        inbound_video_packets_in_window: u64,
        inbound_video_loss_ratio_1s: f64,
        inbound_video_loss_ratio_5s: f64,
        video_rtt_ms: Option<f64>,
        video_rtt_source: Option<String>,
        video_nack_recovery_rtt_ms: Option<f64>,
        video_remb_bps: Option<u32>,
        inbound_video_jitter_ms: Option<f64>,
        video_loss_finalized_packets_in_window: u64,
        video_loss_recovered_packets_in_window: u64,
        video_loss_late_recovered_packets_in_window: u64,
        video_width: Option<u32>,
        video_height: Option<u32>,
        transport_state: XbxEngineTransportStateDto,
    },
    ErrorReported {
        code: String,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEnginePacketGapObservationDto {
    pub observation_id: u64,
    pub expected_sequence: u16,
    pub received_sequence: u16,
    pub missing_count: u16,
    pub source: String,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_packet_count: Option<u16>,
    pub frame_missing_count: Option<u16>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineFrameBudgetDto {
    pub recovery_stage: String,
    pub chain_value: String,
    pub rtt_slack: String,
    pub failure_cost: String,
    pub window_source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineFrameDropObservationDto {
    pub observation_id: u64,
    pub reason: String,
    pub stage: Option<String>,
    pub action: Option<String>,
    pub detail: Option<String>,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_seq: Option<u64>,
    pub frame_recovery_disposition: Option<String>,
    pub frame_unrecoverable_reason: Option<String>,
    pub frame_budget: Option<XbxEngineFrameBudgetDto>,
    pub observed_at_ms: f64,
    pub width: u32,
    pub height: u32,
    pub is_keyframe: bool,
    pub queue_depth: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEnginePipelineCandidateDecisionObservationDto {
    pub decision_id: u64,
    pub state: String,
    pub action: String,
    pub detail: String,
    pub frame_seq: Option<u64>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineFrameRecoveryObservationDto {
    pub observation_id: u64,
    pub action: String,
    pub frame_rtp_timestamp: u32,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub frame_recovery_disposition: String,
    pub frame_unrecoverable_reason: Option<String>,
    pub frame_budget: Option<XbxEngineFrameBudgetDto>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineNackObservationDto {
    pub observation_id: u64,
    pub action: String,
    pub source: String,
    pub first_sequence: u16,
    pub last_sequence: u16,
    pub packet_count: u16,
    pub retry_count: u8,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: Option<String>,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub nack_disposition: Option<String>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub frame_unrecoverable_reason: Option<String>,
    pub frame_budget: Option<XbxEngineFrameBudgetDto>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoEscalationObservationDto {
    pub observation_id: u64,
    pub reason: String,
    pub action: String,
    pub recovery_stage: String,
    pub recovery_chain_value: String,
    pub recovery_failure_cost: String,
    pub recovery_window_source: String,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineRecoveryBudgetSnapshotDto {
    pub recovery_epoch: u64,
    pub keyframe_budget_used: u8,
    pub keyframe_budget_limit: u8,
    pub decoder_reset_budget_used: u8,
    pub decoder_reset_budget_limit: u8,
    pub reconnect_budget_used: u8,
    pub reconnect_budget_limit: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineRecoveryDecisionLedgerObservationDto {
    pub decision_id: u64,
    pub state_before: String,
    pub state_after: String,
    pub input_signal: String,
    pub gate_result: String,
    pub action_selected: String,
    pub budget_before: Option<XbxEngineRecoveryBudgetSnapshotDto>,
    pub budget_after: Option<XbxEngineRecoveryBudgetSnapshotDto>,
    pub command_result: Option<String>,
    pub command_detail: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoTimelineGapSnapshotDto {
    pub state: String,
    pub sequence: Option<u16>,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_importance: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoTimelineFrameSnapshotDto {
    pub state: String,
    pub frame_rtp_timestamp: Option<u32>,
    pub is_keyframe: Option<bool>,
    pub frame_importance: Option<String>,
    pub close_reason: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoTimelineChainSnapshotDto {
    pub state: String,
    pub reason: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoTimelineObservationDto {
    pub observation_id: u64,
    pub source_event: String,
    pub gap: Option<XbxEngineVideoTimelineGapSnapshotDto>,
    pub frame: Option<XbxEngineVideoTimelineFrameSnapshotDto>,
    pub chain: XbxEngineVideoTimelineChainSnapshotDto,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineAnchorCandidateLedgerDto {
    pub recovery_epoch: u64,
    pub frame_rtp_timestamp: Option<u32>,
    pub state: String,
    pub source_event: String,
    pub failure_reason: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoBweObservationDto {
    pub observation_id: u64,
    pub mode: String,
    pub decision_reason: String,
    pub target_remb_kbps: u32,
    pub observed_remb_kbps: Option<u32>,
    pub actual_video_bitrate_kbps: f64,
    pub loss_ratio: f64,
    pub rtt_ms: Option<f64>,
    pub transport_path: Option<String>,
    pub twcc_feedback_interval_ms: Option<f64>,
    pub twcc_observed_packet_count: Option<u16>,
    pub twcc_covered_sequence_span: Option<u16>,
    pub twcc_receive_bitrate_kbps: Option<f64>,
    pub twcc_delivery_ratio: Option<f64>,
    pub twcc_loss_ratio: Option<f64>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineRtcBuilderObservationDto {
    pub observation_id: u64,
    pub controlled_twcc_registry: bool,
    pub feedback_interval_ms: f64,
    pub registered_header_extensions: Vec<String>,
    pub registered_rtcp_feedback: Vec<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineTwccRemoteStreamObservationDto {
    pub observation_id: u64,
    pub ssrc: u32,
    pub mime_type: String,
    pub twcc_ext_id: Option<u8>,
    pub header_extensions: Vec<String>,
    pub rtcp_feedback: Vec<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineRemoteAnswerObservationDto {
    pub observation_id: u64,
    pub video_payload_order: Vec<u8>,
    pub selected_video_payload_type: Option<u8>,
    pub selected_video_mime_type: Option<String>,
    pub selected_video_profile_level_id: Option<String>,
    pub selected_video_h264_sprop_parameter_sets: Option<Vec<String>>,
    pub accepted_video_rtcp_feedback: Vec<String>,
    pub accepted_audio_rtcp_feedback: Vec<String>,
    pub accepted_video_header_extensions: Vec<String>,
    pub accepted_audio_header_extensions: Vec<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineTwccExtensionObservationDto {
    pub observation_id: u64,
    pub state: String,
    pub ssrc: u32,
    pub sequence_number: u16,
    pub expected_ext_id: u8,
    pub packet_seen_count: u64,
    pub missing_count: u64,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoTwccObservationDto {
    pub observation_id: u64,
    pub source: String,
    pub feedback_packet_count: u16,
    pub covered_sequence_start: u16,
    pub covered_sequence_end: u16,
    pub covered_sequence_span: u16,
    pub observed_packet_count: u16,
    pub observed_byte_count: u64,
    pub coverage_ratio: Option<f64>,
    pub ledger_hit_ratio: Option<f64>,
    pub feedback_interval_ms: Option<f64>,
    pub arrival_span_ms: Option<f64>,
    pub receive_bitrate_kbps: Option<f64>,
    pub twcc_sample_valid: bool,
    pub twcc_invalid_reason: Option<String>,
    pub quality: String,
    pub delivery_ratio: f64,
    pub packet_loss_ratio: f64,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XbxEngineBuildFingerprintDto {
    pub git_commit_short: String,
    pub workspace_dirty: bool,
    pub build_timestamp_unix_ms: String,
    pub cargo_profile: String,
    pub default_feedback_interval_ms: u64,
    pub effective_feedback_interval_ms: u64,
    pub controlled_twcc_registry: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineDataChannelMessageCatalogObservationDto {
    pub observation_id: u64,
    pub direction: String,
    pub channel: String,
    pub kind_type: Option<String>,
    pub kind_message: Option<String>,
    pub target: Option<String>,
    pub keys: Vec<String>,
    pub payload_len: usize,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineKeyframeRequestEpisodeObservationDto {
    pub episode_id: u64,
    pub request_reason: Option<String>,
    pub request_kind: Option<String>,
    pub status: String,
    pub requested_at_ms: f64,
    pub sent_at_ms: Option<f64>,
    pub deadline_at_ms: Option<f64>,
    pub first_keyframe_packet_at_ms: Option<f64>,
    pub first_keyframe_decoded_at_ms: Option<f64>,
    pub response_rtp_timestamp: Option<u32>,
    pub response_frame_seq: Option<u64>,
    pub response_verdict: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineH264InspectionObservationDto {
    pub observation_id: u64,
    pub frame_rtp_timestamp: Option<u32>,
    pub nal_types: Vec<String>,
    pub has_inband_sps: bool,
    pub has_inband_pps: bool,
    pub committed_sps_present: bool,
    pub committed_pps_present: bool,
    pub slice_headers_valid: bool,
    pub delta_continuation_ready: bool,
    pub parameter_sets_changed: bool,
    pub config_changed: bool,
    pub is_idr: bool,
    pub bootstrap_ready: bool,
    pub bootstrap_reject_reason: Option<String>,
    pub admission_accepted: bool,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineStatsDto {
    pub resolution: String,
    pub rtt: String,
    pub fps: f64,
    pub runtime_summary: Option<String>,
    pub primary_issue_chain: Option<String>,
    pub latest_decision_summary: Option<String>,
    pub remote_profile_baseline: Option<String>,
    pub remote_profile_dynamic: Option<String>,
    pub remote_profile_effective_label: Option<String>,
    pub session_phase: Option<String>,
    pub transport_strategy_profile: Option<String>,
    pub recovery_strategy_profile: Option<String>,
    pub recovery_diagnosis: Option<String>,
    pub direct_gaming_bitrate_band: Option<String>,
    pub recovery_owner_state: Option<String>,
    pub recovery_owner_reason: Option<String>,
    pub video_owner_source: Option<String>,
    pub video_owner_observed_at_ms: Option<f64>,
    pub video_health: Option<String>,
    pub stall_kind: Option<String>,
    pub inbound_video_fps: Option<f64>,
    pub decode_fps: Option<f64>,
    pub present_fps: Option<f64>,
    pub pl: String,
    pub fl: String,
    pub jit: String,
    pub br: String,
    pub decode: String,
    pub transport_path: Option<String>,
    pub transport_candidate_pair: Option<String>,
    pub transport_protocol: Option<String>,
    pub transport_address_family: Option<String>,
    pub transport_state: Option<String>,
    pub video_rtt_source: Option<String>,
    pub video_remb_bps: Option<u32>,
    pub inbound_bitrate_kbps: Option<f64>,
    pub inbound_video_bitrate_kbps: Option<f64>,
    pub inbound_audio_bitrate_kbps: Option<f64>,
    pub latest_audio_playout_time_ms: Option<f64>,
    pub audio_playout_latency_ms: Option<f64>,
    pub audio_video_playout_delta_ms: Option<f64>,
    pub actual_video_bitrate_source: Option<String>,
    pub video_bwe_mode: Option<String>,
    pub video_bwe_reason: Option<String>,
    pub video_target_remb_kbps: Option<u32>,
    pub video_observed_remb_kbps: Option<u32>,
    pub video_actual_bitrate_kbps: Option<f64>,
    pub video_twcc_receive_bitrate_kbps: Option<f64>,
    pub video_twcc_loss_ratio: Option<f64>,
    pub video_twcc_delivery_ratio: Option<f64>,
    pub video_twcc_feedback_interval_ms: Option<f64>,
    pub twcc_observation_state: Option<String>,
    pub inbound_bytes_total: Option<u64>,
    pub inbound_video_bytes_total: Option<u64>,
    pub inbound_audio_bytes_total: Option<u64>,
    pub inbound_video_packet_count_total: Option<u64>,
    pub latest_video_track_status: Option<XbxEngineVideoTrackStatusDto>,
    pub video_decoder_reset_count: Option<u64>,
    pub video_decoder_stalled: Option<bool>,
    pub video_decoder_hardware_failure_streak: Option<u32>,
    pub latest_video_decoder_hardware_failure_time_ms: Option<f64>,
    pub latest_video_decoder_hardware_failure_status: Option<i32>,
    pub video_decoder_recovery_state: Option<String>,
    pub video_decoder_recovery_event: Option<String>,
    pub video_decoder_recovery_detail: Option<String>,
    pub video_decoder_recovery_status: Option<i32>,
    pub video_decoder_recovery_state_changed_at_ms: Option<f64>,
    pub video_renderer_stalled: Option<bool>,
    pub packet_age_ms: Option<f64>,
    pub decode_age_ms: Option<f64>,
    pub present_age_ms: Option<f64>,
    pub packet_to_decode_ms: Option<f64>,
    pub decode_to_present_ms: Option<f64>,
    pub packet_to_present_ms: Option<f64>,
    pub video_decode_input_drop_count_total: Option<u64>,
    pub video_decode_output_drop_count_total: Option<u64>,
    pub video_pacer_submit_count_total: Option<u64>,
    pub video_pacer_drop_count_total: Option<u64>,
    pub video_renderer_submit_count_total: Option<u64>,
    pub video_renderer_drop_count_total: Option<u64>,
    pub video_present_drop_count_total: Option<u64>,
    pub video_present_overwrite_count_total: Option<u64>,
    pub video_present_submit_count_total: Option<u64>,
    pub host_no_pending_take_count_total: Option<u64>,
    pub host_no_pending_streak: Option<u32>,
    pub host_no_pending_max_streak: Option<u32>,
    pub host_no_pending_pressure_level: Option<String>,
    pub host_display_tick_epoch: Option<u64>,
    pub video_present_epoch: Option<u64>,
    pub host_cadence_phase: Option<String>,
    pub video_present_descriptor_upload_mode: Option<String>,
    pub video_present_descriptor_metal_import_count_total: Option<u64>,
    pub video_present_descriptor_cpu_upload_count_total: Option<u64>,
    pub latest_keyframe_request_episode: Option<XbxEngineKeyframeRequestEpisodeObservationDto>,
    pub latest_h264_inspection_observation: Option<XbxEngineH264InspectionObservationDto>,
    pub recovery_keyframe_request_count: Option<u64>,
    pub recovery_decoder_reset_count: Option<u64>,
    pub recovery_reconnect_count: Option<u64>,
    pub recovery_hard_fallback_timer_ms: Option<f64>,
    pub recovery_hard_fallback_trigger_reason: Option<String>,
    pub recovery_hard_fallback_timer_reset_reason: Option<String>,
    pub last_recovery_action: Option<String>,
    pub last_recovery_action_at_ms: Option<f64>,
    pub last_recovery_reason: Option<String>,
    /// policy | runtime | other — 与宿主侧 `restart=true` 重协商观测对齐
    #[serde(default)]
    pub reconnect_trigger_source: Option<String>,
    pub latest_decode_candidate_decision: Option<XbxEnginePipelineCandidateDecisionObservationDto>,
    pub latest_render_candidate_decision: Option<XbxEnginePipelineCandidateDecisionObservationDto>,
    pub latest_video_packet_gap: Option<XbxEnginePacketGapObservationDto>,
    pub latest_video_frame_drop: Option<XbxEngineFrameDropObservationDto>,
    pub latest_video_frame_recovery_observation: Option<XbxEngineFrameRecoveryObservationDto>,
    pub latest_video_nack_observation: Option<XbxEngineNackObservationDto>,
    pub latest_video_escalation_observation: Option<XbxEngineVideoEscalationObservationDto>,
    pub latest_recovery_decision_ledger: Option<XbxEngineRecoveryDecisionLedgerObservationDto>,
    pub latest_video_timeline_observation: Option<XbxEngineVideoTimelineObservationDto>,
    pub latest_anchor_candidate_ledger: Option<XbxEngineAnchorCandidateLedgerDto>,
    pub latest_video_bwe_observation: Option<XbxEngineVideoBweObservationDto>,
    pub latest_video_twcc_observation: Option<XbxEngineVideoTwccObservationDto>,
    pub latest_rtc_builder_observation: Option<XbxEngineRtcBuilderObservationDto>,
    pub latest_twcc_remote_stream_observation: Option<XbxEngineTwccRemoteStreamObservationDto>,
    pub latest_remote_answer_observation: Option<XbxEngineRemoteAnswerObservationDto>,
    pub latest_twcc_extension_observation: Option<XbxEngineTwccExtensionObservationDto>,
    pub latest_data_channel_message_catalog_observation:
        Option<XbxEngineDataChannelMessageCatalogObservationDto>,
    pub latest_observation_label: Option<String>,
    pub latest_observation_summary: Option<String>,
    pub latest_target_remb_action: Option<String>,
    pub latest_target_remb_summary: Option<String>,
    pub build_fingerprint: Option<XbxEngineBuildFingerprintDto>,
}
