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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XbxEnginePresentationMilestoneDto {
    Idle,
    Connected,
    MediaReady,
    Degraded,
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
    /// `MediaVideoReady` 保持历史兼容语义：只表示媒体协商/尺寸 ready。
    MediaVideoReady {
        width: u32,
        height: u32,
    },
    PresentationMilestoneChanged {
        milestone: XbxEnginePresentationMilestoneDto,
        connected_at_ms: Option<f64>,
        media_ready_at_ms: Option<f64>,
        stage: Option<String>,
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
    /// 首帧时长专用观测：首帧真实渲染落地时发出一次，用于启动/重连首帧慢诊断。
    FirstFrameLatencyObserved {
        connected_at_ms: Option<f64>,
        first_packet_at_ms: Option<f64>,
        first_decode_at_ms: Option<f64>,
        first_render_at_ms: f64,
        from_connected_to_first_render_ms: Option<f64>,
        from_first_packet_to_first_render_ms: Option<f64>,
        from_first_decode_to_first_render_ms: Option<f64>,
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
pub struct XbxEngineReplacementDecisionObservationDto {
    #[serde(default)]
    pub dropped_frame_seq: Option<u64>,
    #[serde(default)]
    pub dropped_rtp_timestamp: Option<u32>,
    #[serde(default)]
    pub dropped_presentation_value_role: Option<String>,
    #[serde(default)]
    pub kept_frame_seq: Option<u64>,
    #[serde(default)]
    pub kept_rtp_timestamp: Option<u32>,
    #[serde(default)]
    pub kept_presentation_value_role: Option<String>,
    #[serde(default)]
    pub same_recovery_epoch: Option<bool>,
    #[serde(default)]
    pub same_recovery_owner_chain: Option<bool>,
    #[serde(default)]
    pub supersede_reason: Option<String>,
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
    #[serde(default)]
    pub replacement_decision: Option<XbxEngineReplacementDecisionObservationDto>,
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
    #[serde(default)]
    pub replacement_decision: Option<XbxEngineReplacementDecisionObservationDto>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoDecoderProbeObservationDto {
    pub observation_id: u64,
    pub selected_backend_name: String,
    pub selected_backend_kind: String,
    pub fallback_count: u32,
    pub fallback_summary: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoDecoderBootstrapGateObservationDto {
    pub observation_id: u64,
    pub recovery_state: String,
    pub frame_rtp_timestamp: u32,
    pub is_idr: bool,
    pub has_inband_sps: bool,
    pub has_inband_pps: bool,
    pub committed_sps_present: bool,
    pub committed_pps_present: bool,
    pub bootstrap_ready: bool,
    pub bootstrap_reject_reason: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineDecodeOutputPathObservationDto {
    pub observation_id: u64,
    pub verdict: String,
    pub detail: String,
    pub frame_rtp_timestamp: u32,
    pub is_keyframe: bool,
    pub status: Option<i32>,
    pub send_packet_status: Option<i32>,
    pub receive_frame_status: Option<i32>,
    pub backend_no_output_streak: Option<u32>,
    pub input_frames_since_last_decoded: Option<u32>,
    pub bootstrap_reject_reason: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineRemoteFrameCaptureObservationDto {
    pub observation_id: u64,
    pub trigger: String,
    pub backend_name: String,
    pub frame_rtp_timestamp: u32,
    pub is_keyframe: bool,
    pub width: u32,
    pub height: u32,
    pub payload_bytes: usize,
    pub payload_fingerprint: u64,
    pub payload_prefix_hex: String,
    pub nal_types: Vec<String>,
    pub nal_count: u16,
    pub has_inband_sps: bool,
    pub has_inband_pps: bool,
    pub bootstrap_ready: bool,
    pub bootstrap_reject_reason: Option<String>,
    pub parameter_sets_changed: bool,
    pub config_changed: bool,
    pub slice_headers_valid: bool,
    pub send_packet_status: Option<i32>,
    pub receive_frame_status: Option<i32>,
    pub status: Option<i32>,
    pub backend_no_output_streak: Option<u32>,
    pub input_frames_since_last_decoded: Option<u32>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineFrameRecoveryObservationDto {
    pub observation_id: u64,
    pub action: String,
    pub frame_rtp_timestamp: u32,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub frame_recovery_disposition: Option<String>,
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
    pub frame_value: Option<String>,
    pub gap_severity: Option<String>,
    pub repairability: Option<f64>,
    pub recovery_episode_stage: Option<String>,
    pub recovery_episode_progress_at_ms: Option<f64>,
    pub coalescing_mode: Option<String>,
    pub unlock_reason: Option<String>,
    pub preempt_reason: Option<String>,
    pub recovery_primary_action: Option<String>,
    pub owner_surface_state: Option<String>,
    pub anchor_evidence: Option<String>,
    pub keyframe_episode_health: Option<String>,
    pub escalation_basis: Option<String>,
    pub budget_before: Option<XbxEngineRecoveryBudgetSnapshotDto>,
    pub budget_after: Option<XbxEngineRecoveryBudgetSnapshotDto>,
    pub trigger_observation_label: Option<String>,
    pub trigger_observation_summary: Option<String>,
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
    #[serde(default)]
    pub budget_importance: Option<String>,
    #[serde(default)]
    pub evidence_importance: Option<String>,
    #[serde(default)]
    pub gap_dependency_confidence: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoTimelineFrameSnapshotDto {
    pub state: String,
    pub frame_rtp_timestamp: Option<u32>,
    pub is_keyframe: Option<bool>,
    pub frame_importance: Option<String>,
    #[serde(default)]
    pub budget_importance: Option<String>,
    #[serde(default)]
    pub evidence_importance: Option<String>,
    pub close_reason: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoTimelineChainSnapshotDto {
    pub state: String,
    pub reason: Option<String>,
    #[serde(default)]
    pub chain_break_evidence: Option<String>,
    pub observed_at_ms: f64,
}

/// receiver-local 接收观测（pre-decode）；不驱动全局 recovery owner。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoReceiverObservationDto {
    pub observation_id: u64,
    pub receiver_state: String,
    pub gap_sequence: Option<u16>,
    pub gap_span: Option<u16>,
    pub nack_in_flight: bool,
    pub keyframe_request_pending: bool,
    pub bootstrap_reject_reason: Option<String>,
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
    #[serde(default)]
    pub status_detail: Option<String>,
    pub requested_at_ms: f64,
    pub sent_at_ms: Option<f64>,
    pub deadline_at_ms: Option<f64>,
    #[serde(default)]
    pub transport_detail: Option<String>,
    #[serde(default)]
    pub first_video_packet_at_ms: Option<f64>,
    #[serde(default)]
    pub first_video_packet_rtp_timestamp: Option<u32>,
    #[serde(default)]
    pub first_video_packet_is_keyframe: Option<bool>,
    pub first_keyframe_packet_at_ms: Option<f64>,
    pub first_keyframe_decoded_at_ms: Option<f64>,
    pub response_rtp_timestamp: Option<u32>,
    pub response_frame_seq: Option<u64>,
    pub response_verdict: Option<String>,
    #[serde(default)]
    pub lifecycle_phase: Option<String>,
    #[serde(default)]
    pub retired_at_ms: Option<f64>,
    #[serde(default)]
    pub family_id: Option<String>,
    #[serde(default)]
    pub owner_episode_id: Option<u64>,
    #[serde(default)]
    pub suppress_duration_ms: Option<f64>,
    #[serde(default)]
    pub release_reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineH264InspectionObservationDto {
    pub observation_id: u64,
    pub frame_rtp_timestamp: Option<u32>,
    pub nal_types: Vec<String>,
    #[serde(default)]
    pub nal_count: u16,
    #[serde(default)]
    pub vcl_nal_count: u16,
    pub has_inband_sps: bool,
    pub has_inband_pps: bool,
    pub committed_sps_present: bool,
    pub committed_pps_present: bool,
    pub slice_headers_valid: bool,
    pub delta_continuation_ready: bool,
    pub parameter_sets_changed: bool,
    pub config_changed: bool,
    pub is_idr: bool,
    #[serde(default)]
    pub sample_width: Option<u32>,
    #[serde(default)]
    pub sample_height: Option<u32>,
    pub bootstrap_ready: bool,
    pub bootstrap_reject_reason: Option<String>,
    #[serde(default)]
    pub continuation_verdict: Option<String>,
    pub admission_accepted: bool,
    pub observed_at_ms: f64,
    /// 生成观测时绑定的 keyframe episode（避免 trace 层二次推断失真）。
    #[serde(default)]
    pub bound_episode_id: Option<u64>,
    #[serde(default)]
    pub bound_episode_status: Option<String>,
    #[serde(default)]
    pub bound_as_recovery_response: Option<bool>,
    #[serde(default)]
    pub bound_response_rtp_timestamp: Option<u32>,
    #[serde(default)]
    pub bound_recovery_epoch: Option<u64>,
    #[serde(default)]
    pub episode_phase_at_observation: Option<String>,
    #[serde(default)]
    pub is_post_recovery_degradation: Option<bool>,
    #[serde(default)]
    pub reject_classification: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XbxEnginePictureRecoveryTransitionObservationDto {
    pub observation_id: u64,
    pub episode_id: Option<u64>,
    pub recovery_epoch: Option<u64>,
    pub phase: String,
    pub from_phase: Option<String>,
    pub to_phase: String,
    pub cause: Option<String>,
    pub detail: Option<String>,
    pub rtp_timestamp: Option<u32>,
    pub frame_seq: Option<u64>,
    pub owner_state: Option<String>,
    pub transport_state: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XbxEnginePictureRecoveryBlockerObservationDto {
    pub observation_id: u64,
    pub episode_id: Option<u64>,
    pub recovery_epoch: Option<u64>,
    pub gate: String,
    pub blocker_kind: String,
    pub severity: String,
    pub first_observed_at_ms: f64,
    pub observed_at_ms: f64,
    pub count: u32,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_seq: Option<u64>,
    pub owner_state: Option<String>,
    pub transport_state: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineVideoIngressTerminationObservationDto {
    pub observation_id: u64,
    pub termination_id: u64,
    pub derived_from_termination_id: Option<u64>,
    pub kind: String,
    pub cause: String,
    pub upstream_cause: Option<String>,
    pub source_subsystem: Option<String>,
    pub linked_recovery_epoch: Option<u64>,
    pub linked_episode_id: Option<u64>,
    pub transport_state: Option<String>,
    pub owner_state: Option<String>,
    pub video_track_state: Option<String>,
    pub recent_command: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineFirstFrameLatencyObservationDto {
    pub observation_id: u64,
    pub episode_id: Option<u64>,
    pub recovery_epoch: Option<u64>,
    pub control_ready_to_pli_sent_ms: Option<f64>,
    pub pli_sent_to_first_idr_packet_ms: Option<f64>,
    pub first_idr_packet_to_first_decode_ms: Option<f64>,
    pub first_decode_to_clean_anchor_committed_ms: Option<f64>,
    pub clean_anchor_committed_to_display_stable_ms: Option<f64>,
    pub terminal_phase: Option<String>,
    pub incomplete_reason: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XbxEngineStatsDto {
    pub resolution: String,
    pub rtt: String,
    pub fps: f64,
    pub stream_lifecycle_phase: Option<String>,
    pub presentation_milestone: Option<String>,
    pub connected_milestone_elapsed_ms: Option<f64>,
    pub media_ready_milestone_elapsed_ms: Option<f64>,
    pub presentation_failed_stage: Option<String>,
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
    /// 与 `MediaRuntimeStats.recovery_rfc_authoritative_fault_domain` 同源；不拼入 `recovery_diagnosis`。
    #[serde(default)]
    pub recovery_rfc_fault_domain: Option<String>,
    /// 与 `recovery_rfc_authoritative_stage` 同源。
    #[serde(default)]
    pub recovery_rfc_stage: Option<String>,
    /// 与 `recovery_rfc_authoritative_ceiling` 同源。
    #[serde(default)]
    pub recovery_rfc_ceiling: Option<String>,
    #[serde(default)]
    pub recovery_playback_recovered_at_ms: Option<f64>,
    #[serde(default)]
    pub recovery_playback_recovered_phase: Option<String>,
    #[serde(default)]
    pub recovery_fresh_anchor_recovered_at_ms: Option<f64>,
    #[serde(default)]
    pub recovery_displayed_idr_rtp: Option<u32>,
    #[serde(default)]
    pub recovery_displayed_idr_at_ms: Option<f64>,
    #[serde(default)]
    pub recovery_effective_rtt_ms: Option<f64>,
    #[serde(default)]
    pub recovery_dynamic_nack_timeout_ms: Option<f64>,
    #[serde(default)]
    pub recovery_dynamic_nack_retry_interval_ms: Option<f64>,
    #[serde(default)]
    pub recovery_dynamic_pli_refresh_interval_ms: Option<f64>,
    #[serde(default)]
    pub recovery_dynamic_fir_retry_interval_ms: Option<f64>,
    #[serde(default)]
    pub recovery_dynamic_decoded_pending_commit_hold_ms: Option<f64>,
    #[serde(default)]
    pub recovery_dynamic_continuation_patience_ms: Option<f64>,
    #[serde(default)]
    pub recovery_dynamic_clean_anchor_patience_ms: Option<f64>,
    #[serde(default)]
    pub recovery_codec_bootstrap_salvage_applied: Option<bool>,
    #[serde(default)]
    pub recovery_codec_bootstrap_salvage_failed_reason: Option<String>,
    #[serde(default)]
    pub recovery_nack_first_attempt_survival_window_ms: Option<f64>,
    #[serde(default)]
    pub recovery_nack_first_attempt_deadline_at_ms: Option<f64>,
    #[serde(default)]
    pub recovery_nack_first_attempt_still_economical: Option<bool>,
    #[serde(default)]
    pub recovery_nack_retry_allowed_reason: Option<String>,
    #[serde(default)]
    pub recovery_nack_retry_suppressed_reason: Option<String>,
    pub direct_gaming_bitrate_band: Option<String>,
    pub recovery_owner_state: Option<String>,
    #[serde(default)]
    pub recovery_owner_contract_state: Option<String>,
    pub recovery_owner_reason: Option<String>,
    pub video_owner_source: Option<String>,
    pub video_owner_observed_at_ms: Option<f64>,
    /// 派生恢复表面：`steady` / `repairing` / `await-idr` / `supply-break`。
    #[serde(default)]
    pub recovery_surface_phase: Option<String>,
    /// 媒体供给主相位（L0 单轨）。
    #[serde(default)]
    pub media_supply_phase: Option<String>,
    /// receive-local keyframe 尝试序号（trace `keyframeRequestOutcome`）。
    #[serde(default)]
    pub keyframe_request_outcome_seq: u64,
    /// 派生解码健康（owner 决策用）。
    #[serde(default)]
    pub derived_decoder_health: Option<String>,
    pub video_health: Option<String>,
    #[serde(default)]
    pub chain_health: Option<String>,
    #[serde(default)]
    pub presentation_health: Option<String>,
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
    #[serde(default)]
    pub latest_video_packet_arrival_rtp_timestamp: Option<u32>,
    pub latest_video_track_status: Option<XbxEngineVideoTrackStatusDto>,
    pub video_decoder_reset_count: Option<u64>,
    pub video_decoder_stalled: Option<bool>,
    pub latest_video_decoder_probe_observation: Option<XbxEngineVideoDecoderProbeObservationDto>,
    pub latest_video_decoder_bootstrap_gate_observation:
        Option<XbxEngineVideoDecoderBootstrapGateObservationDto>,
    pub latest_decode_output_path_observation: Option<XbxEngineDecodeOutputPathObservationDto>,
    pub latest_remote_frame_capture_observation: Option<XbxEngineRemoteFrameCaptureObservationDto>,
    pub video_decoder_hardware_failure_streak: Option<u32>,
    pub latest_video_decoder_hardware_failure_time_ms: Option<f64>,
    pub latest_video_decoder_hardware_failure_status: Option<i32>,
    pub video_decoder_recovery_state: Option<String>,
    pub video_decoder_recovery_event: Option<String>,
    pub video_decoder_recovery_detail: Option<String>,
    pub video_decoder_recovery_status: Option<i32>,
    pub video_decoder_recovery_state_changed_at_ms: Option<f64>,
    #[serde(default)]
    pub latest_video_decode_ok_rtp_timestamp: Option<u32>,
    #[serde(default)]
    pub latest_video_decode_ok_time_ms: Option<f64>,
    #[serde(default)]
    pub video_renderer_stalled: Option<bool>,
    #[serde(default)]
    pub video_renderer_stall_blocks_presentation: Option<bool>,
    pub packet_age_ms: Option<f64>,
    pub decode_age_ms: Option<f64>,
    pub present_age_ms: Option<f64>,
    pub packet_to_decode_ms: Option<f64>,
    pub decode_to_present_ms: Option<f64>,
    #[serde(default)]
    pub submit_to_present_ms: Option<f64>,
    pub packet_to_present_ms: Option<f64>,
    #[serde(default)]
    pub inspection_pulse_active: Option<bool>,
    pub video_decode_input_drop_count_total: Option<u64>,
    pub video_decode_output_drop_count_total: Option<u64>,
    pub video_pacer_submit_count_total: Option<u64>,
    pub video_pacer_drop_count_total: Option<u64>,
    pub video_renderer_submit_count_total: Option<u64>,
    pub video_renderer_drop_count_total: Option<u64>,
    pub host_mailbox_drop_count_total: Option<u64>,
    pub host_mailbox_overwrite_count_total: Option<u64>,
    pub host_mailbox_enqueue_count_total: Option<u64>,
    pub host_no_pending_take_count_total: Option<u64>,
    pub host_no_pending_streak: Option<u32>,
    pub host_no_pending_max_streak: Option<u32>,
    pub host_no_pending_pressure_level: Option<String>,
    #[serde(default)]
    pub host_mailbox_submit_epoch: Option<u64>,
    pub host_display_tick_epoch: Option<u64>,
    #[serde(default)]
    pub host_frame_present_epoch: Option<u64>,
    pub host_cadence_phase: Option<String>,
    #[serde(default)]
    pub latest_host_mailbox_submit_time_ms: Option<f64>,
    #[serde(default)]
    pub latest_video_host_submit_rtp_timestamp: Option<u32>,
    #[serde(default)]
    pub latest_video_host_present_time_ms: Option<f64>,
    #[serde(default)]
    pub submit_age_ms: Option<f64>,
    #[serde(default)]
    pub display_age_ms: Option<f64>,
    #[serde(default)]
    pub host_view_generation: Option<u64>,
    #[serde(default)]
    pub latest_host_view_created_at_ms: Option<f64>,
    #[serde(default)]
    pub last_displayed_frame_seq: Option<u64>,
    #[serde(default)]
    pub last_displayed_frame_rtp_timestamp: Option<u32>,
    #[serde(default)]
    pub last_displayed_at_ms: Option<f64>,
    pub video_present_descriptor_upload_mode: Option<String>,
    pub video_present_descriptor_metal_import_count_total: Option<u64>,
    pub video_present_descriptor_cpu_upload_count_total: Option<u64>,
    #[serde(default)]
    pub latest_feedback_target_availability_state: Option<String>,
    #[serde(default)]
    pub latest_feedback_target_availability_reason: Option<String>,
    #[serde(default)]
    pub latest_feedback_target_availability_target: Option<String>,
    #[serde(default)]
    pub latest_feedback_target_availability_observed_at_ms: Option<f64>,
    #[serde(default)]
    pub latest_video_rtcp_send_failure_time_ms: Option<f64>,
    #[serde(default)]
    pub latest_video_rtcp_send_failure_reason: Option<String>,
    pub latest_keyframe_request_episode: Option<XbxEngineKeyframeRequestEpisodeObservationDto>,
    /// receive 侧最近一次 keyframe 请求来源（trace `keyframeRequestOutcome`）。
    #[serde(default)]
    pub latest_keyframe_request_source: Option<String>,
    /// receive 侧最近一次 keyframe 请求结果（trace `keyframeRequestOutcome`）。
    #[serde(default)]
    pub latest_keyframe_request_outcome: Option<String>,
    #[serde(default)]
    pub latest_insert_decision: Option<String>,
    #[serde(default)]
    pub latest_insert_decision_reason: Option<String>,
    #[serde(default)]
    pub insert_decode_bypass_aligned: Option<bool>,
    #[serde(default)]
    pub insert_hold_decode_bypass_mismatch_total: Option<u64>,
    #[serde(default)]
    pub recovery_picture_recovery_authority: Option<String>,
    #[serde(default)]
    pub recovery_picture_recovery_delegated_total: Option<u64>,
    #[serde(default)]
    pub recovery_session_keyframe_in_flight: Option<bool>,
    #[serde(default)]
    pub receive_sparse_idr_pli_interval_ms: Option<f64>,
    #[serde(default)]
    pub ingress_waiting_rtp_marker_total: Option<u64>,
    #[serde(default)]
    pub ingress_waiting_idr_inspection_total: Option<u64>,
    #[serde(default)]
    pub ingress_idr_not_admitted_total: Option<u64>,
    #[serde(default)]
    pub latest_ingress_idr_not_admitted_reason: Option<String>,
    /// 解码器参考链与 bootstrap IDR 对齐时刻（ms）。
    #[serde(default)]
    pub recovery_decoder_reference_synced_at_ms: Option<f64>,
    /// 近期 keyframe 请求 episode 历史，供诊断与 H264 观测绑定；默认空。
    #[serde(default)]
    pub recent_keyframe_request_episodes: Vec<XbxEngineKeyframeRequestEpisodeObservationDto>,
    pub latest_h264_inspection_observation: Option<XbxEngineH264InspectionObservationDto>,
    #[serde(default)]
    pub latest_picture_recovery_transition_observation:
        Option<XbxEnginePictureRecoveryTransitionObservationDto>,
    #[serde(default)]
    pub latest_picture_recovery_blocker_observation:
        Option<XbxEnginePictureRecoveryBlockerObservationDto>,
    #[serde(default)]
    pub latest_video_ingress_termination_observation:
        Option<XbxEngineVideoIngressTerminationObservationDto>,
    #[serde(default)]
    pub latest_first_frame_latency_observation: Option<XbxEngineFirstFrameLatencyObservationDto>,
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
    /// runtime 侧连续未取到 render 帧的 tick 计数。
    #[serde(default)]
    pub host_present_take_empty_streak: Option<u32>,
    /// 最近一次成功从 render mailbox 取到帧并提交给 host 的时间（ms）。
    #[serde(default)]
    pub host_mailbox_latest_submit_at_ms: Option<f64>,
    /// ICE candidate policy 观测（与 webrtc_direct 对齐）
    #[serde(default)]
    pub ice_policy_mode: Option<String>,
    #[serde(default)]
    pub ice_policy_digest: Option<String>,
    #[serde(default)]
    pub ice_policy_source: Option<String>,
    #[serde(default)]
    pub ice_policy_filtered_count: Option<u32>,
    #[serde(default)]
    pub ice_policy_derived_count: Option<u32>,
    #[serde(default)]
    pub ice_policy_skipped_by_family_mismatch_count: Option<u32>,
    pub latest_decode_candidate_decision: Option<XbxEnginePipelineCandidateDecisionObservationDto>,
    pub latest_render_mailbox_decision: Option<XbxEnginePipelineCandidateDecisionObservationDto>,
    pub latest_video_packet_gap: Option<XbxEnginePacketGapObservationDto>,
    pub latest_video_frame_drop: Option<XbxEngineFrameDropObservationDto>,
    pub latest_video_frame_recovery_observation: Option<XbxEngineFrameRecoveryObservationDto>,
    pub latest_video_nack_observation: Option<XbxEngineNackObservationDto>,
    pub latest_video_escalation_observation: Option<XbxEngineVideoEscalationObservationDto>,
    pub latest_recovery_decision_ledger: Option<XbxEngineRecoveryDecisionLedgerObservationDto>,
    pub latest_video_timeline_observation: Option<XbxEngineVideoTimelineObservationDto>,
    pub latest_video_receiver_observation: Option<XbxEngineVideoReceiverObservationDto>,
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
