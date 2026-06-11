use xbxengine_protocol::XbxEngineTransportStateDto;

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineReplacementDecisionObservation {
    pub dropped_frame_seq: Option<u64>,
    pub dropped_rtp_timestamp: Option<u32>,
    pub dropped_presentation_value_role: Option<String>,
    pub kept_frame_seq: Option<u64>,
    pub kept_rtp_timestamp: Option<u32>,
    pub kept_presentation_value_role: Option<String>,
    pub same_recovery_epoch: Option<bool>,
    pub same_recovery_owner_chain: Option<bool>,
    pub supersede_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoPacketGapObservation {
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

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineFrameBudgetObservation {
    pub recovery_stage: String,
    pub chain_value: String,
    pub rtt_slack: String,
    pub failure_cost: String,
    pub window_source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoFrameDropObservation {
    pub observation_id: u64,
    pub reason: String,
    pub stage: Option<String>,
    pub action: Option<String>,
    pub detail: Option<String>,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_seq: Option<u64>,
    pub frame_recovery_disposition: Option<String>,
    pub frame_unrecoverable_reason: Option<String>,
    pub frame_budget: Option<XbxEngineFrameBudgetObservation>,
    pub replacement_decision: Option<XbxEngineReplacementDecisionObservation>,
    pub observed_at_ms: f64,
    pub width: u32,
    pub height: u32,
    pub is_keyframe: bool,
    pub queue_depth: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEnginePipelineCandidateDecisionObservation {
    pub decision_id: u64,
    pub state: String,
    pub action: String,
    pub detail: String,
    pub frame_seq: Option<u64>,
    pub replacement_decision: Option<XbxEngineReplacementDecisionObservation>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoDecoderProbeObservation {
    pub observation_id: u64,
    pub selected_backend_name: String,
    pub selected_backend_kind: String,
    pub fallback_count: u32,
    pub fallback_summary: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoDecoderBootstrapGateObservation {
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

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineDecodeOutputPathObservation {
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

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineRemoteFrameCaptureObservation {
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

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineHostVideoFrameDropEvent {
    pub stage: Option<String>,
    pub action: Option<String>,
    pub detail: Option<String>,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_seq: Option<u64>,
    pub frame_recovery_disposition: Option<String>,
    pub frame_unrecoverable_reason: Option<String>,
    pub observed_at_ms: f64,
    pub width: u32,
    pub height: u32,
    pub is_keyframe: bool,
    pub queue_depth: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XbxEngineHostVideoPresentMetrics {
    /// 宿主 submit/enqueue 最近发生时间（毫秒时间戳）。
    pub latest_host_submit_time_ms: Option<f64>,
    /// 最近一次 submit 对应的帧 RTP 时间戳；用于配对 submit/present 时序。
    pub latest_host_submit_rtp_timestamp: Option<u32>,
    /// 宿主真实 present 发生时间（毫秒时间戳）。
    /// 该字段是 runtime 中 present freshness 的唯一事实源。
    pub latest_host_present_time_ms: Option<f64>,
    /// 宿主 view / layer 重建代次；present 断档时用于区分“旧 view 卡住”和“新 view 待补帧”。
    pub host_view_generation: u64,
    pub latest_host_view_created_at_ms: Option<f64>,
    pub host_mailbox_submit_epoch: u64,
    pub host_display_tick_epoch: u64,
    pub host_frame_present_epoch: u64,
    pub cadence_phase: Option<String>,
    pub present_fps: f64,
    pub host_mailbox_enqueue_count_total: u64,
    pub host_mailbox_drop_count_total: u64,
    pub host_mailbox_overwrite_count_total: u64,
    pub no_pending_take_count_total: u64,
    pub no_pending_streak: u32,
    pub no_pending_max_streak: u32,
    pub descriptor_upload_mode: Option<String>,
    pub descriptor_metal_import_count_total: u64,
    pub descriptor_cpu_upload_count_total: u64,
    pub last_displayed_frame_seq: Option<u64>,
    pub last_displayed_frame_rtp_timestamp: Option<u32>,
    pub last_displayed_at_ms: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineFrameRecoveryObservation {
    pub observation_id: u64,
    pub action: String,
    pub frame_rtp_timestamp: u32,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub frame_recovery_disposition: Option<String>,
    pub frame_unrecoverable_reason: Option<String>,
    pub frame_budget: Option<XbxEngineFrameBudgetObservation>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoNackObservation {
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
    pub frame_budget: Option<XbxEngineFrameBudgetObservation>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoEscalationObservation {
    pub observation_id: u64,
    pub reason: String,
    pub action: String,
    pub recovery_stage: String,
    pub recovery_chain_value: String,
    pub recovery_failure_cost: String,
    pub recovery_window_source: String,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineRecoveryBudgetSnapshot {
    pub recovery_epoch: u64,
    pub keyframe_budget_used: u8,
    pub keyframe_budget_limit: u8,
    pub decoder_reset_budget_used: u8,
    pub decoder_reset_budget_limit: u8,
    pub reconnect_budget_used: u8,
    pub reconnect_budget_limit: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XbxEngineRecoveryDecisionLedgerObservation {
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
    /// RFC 2026-05-13：owner 表面状态（suspect / await-anchor / local-recovery / connectivity-recovery）。
    pub owner_surface_state: Option<String>,
    /// RFC 2026-05-13：锚点证据摘要。
    pub anchor_evidence: Option<String>,
    /// RFC 2026-05-13：IDR episode 健康度（waiting-response / continuation-only / stalled）。
    pub keyframe_episode_health: Option<String>,
    /// RFC 2026-05-13：升级依据（local_supply / anchor_missing / connectivity_bad）。
    pub escalation_basis: Option<String>,
    pub budget_before: Option<XbxEngineRecoveryBudgetSnapshot>,
    pub budget_after: Option<XbxEngineRecoveryBudgetSnapshot>,
    pub trigger_observation_label: Option<String>,
    pub trigger_observation_summary: Option<String>,
    pub command_result: Option<String>,
    pub command_detail: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoTimelineGapSnapshot {
    pub state: String,
    pub sequence: Option<u16>,
    pub frame_rtp_timestamp: Option<u32>,
    /// 兼容旧 trace：与 `evidence_importance` 一致时表示媒体/因果侧 importance。
    pub frame_importance: Option<String>,
    /// NACK/预算调度侧 importance（`link_value` 映射），不等价于媒体断链证据。
    pub budget_importance: Option<String>,
    /// gap 归属帧上的媒体因果 importance（IDR / 参数集变更 / delta）。
    pub evidence_importance: Option<String>,
    /// `bound` / `anonymous` / `inferred`：缺洞是否已绑定具体 RTP 帧时间戳。
    pub gap_dependency_confidence: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoTimelineFrameSnapshot {
    pub state: String,
    pub frame_rtp_timestamp: Option<u32>,
    pub is_keyframe: Option<bool>,
    pub frame_importance: Option<String>,
    pub budget_importance: Option<String>,
    pub evidence_importance: Option<String>,
    pub close_reason: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoTimelineChainSnapshot {
    pub state: String,
    pub reason: Option<String>,
    /// 最近一次进入 `broken` 时可归因的证据标签（如 `boundReferenceGapExpired`）。
    pub chain_break_evidence: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoTimelineObservation {
    pub observation_id: u64,
    pub source_event: String,
    pub gap: Option<XbxEngineVideoTimelineGapSnapshot>,
    pub frame: Option<XbxEngineVideoTimelineFrameSnapshot>,
    pub chain: XbxEngineVideoTimelineChainSnapshot,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoReceiverObservation {
    pub observation_id: u64,
    pub receiver_state: String,
    pub gap_sequence: Option<u16>,
    pub gap_span: Option<u16>,
    pub nack_in_flight: bool,
    pub keyframe_request_pending: bool,
    pub bootstrap_reject_reason: Option<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XbxEngineAnchorCandidateState {
    Observed,
    AwaitingRecovery,
    Repaired,
    Rejected,
    SubmittedCleanAnchor,
}

impl XbxEngineAnchorCandidateState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::AwaitingRecovery => "awaiting-recovery",
            Self::Repaired => "repaired",
            Self::Rejected => "rejected",
            Self::SubmittedCleanAnchor => "submitted-clean-anchor",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XbxEngineAnchorCandidateFailureReason {
    /// 本地 NACK 修补在途；语义上属于 `nack_pending`，不是等关键帧。
    LocalRepairPending,
    AwaitingRecoveryKeyframe,
    InspectionRejectedMissingSps,
    InspectionRejectedMissingPps,
    InspectionRejectedInvalidSliceHeader,
    ChainBrokenReferenceUnrecoverable,
    /// 传输层低价值准入放弃，不表示参考链已断（`as_str` 仍为历史观测键）。
    TransportLowValueCloudHighRttAdmission,
    TransportLowValueDisplayStarvedAdmission,
    /// Recovery 期 supply 近 deadline 的时效性跳过，非 reference-chain 证据。
    TransportTimingNearDeadlineSupplyRecovery,
    GapExpiredDeadline,
    Unknown,
}

impl XbxEngineAnchorCandidateFailureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalRepairPending => "localRepairPending",
            Self::AwaitingRecoveryKeyframe => "awaitingRecoveryAnchor",
            Self::InspectionRejectedMissingSps => "bootstrapMissingSps",
            Self::InspectionRejectedMissingPps => "bootstrapMissingPps",
            Self::InspectionRejectedInvalidSliceHeader => "inspectionRejectInvalidSliceHeader",
            Self::ChainBrokenReferenceUnrecoverable => "referenceChainUnrecoverable",
            Self::TransportLowValueCloudHighRttAdmission => "cloudHighRttLowValueAdmission",
            Self::TransportLowValueDisplayStarvedAdmission => "displayStarvedLowValueAdmission",
            Self::TransportTimingNearDeadlineSupplyRecovery => {
                "estimatedArrivalNearDeadlineSupplyRecovery"
            }
            Self::GapExpiredDeadline => "gapExpiredDeadline",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineAnchorCandidateLedger {
    pub recovery_epoch: u64,
    pub frame_rtp_timestamp: Option<u32>,
    pub state: XbxEngineAnchorCandidateState,
    pub source_event: String,
    pub failure_reason: Option<XbxEngineAnchorCandidateFailureReason>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XbxEngineRecoveryReasonDomain {
    ConnectivityTransport,
    Local,
    Unknown,
}

impl XbxEngineRecoveryReasonDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConnectivityTransport => "connectivity-transport",
            Self::Local => "local",
            Self::Unknown => "unknown",
        }
    }

    pub fn allows_runtime_reconnect_candidate(self) -> bool {
        matches!(self, Self::ConnectivityTransport)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XbxEnginePendingRuntimeRecoveryAction {
    RequestReconnectCandidate {
        observation_id: u64,
        reason: String,
        reason_domain: XbxEngineRecoveryReasonDomain,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoBweObservation {
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

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineIceConnectivityProbeObservation {
    pub candidate_pair_count: u16,
    pub nominated_pair_count: u16,
    pub succeeded_pair_count: u16,
    pub in_progress_pair_count: u16,
    pub failed_pair_count: u16,
    pub max_requests_sent: u64,
    pub max_responses_received: u64,
    pub responses_received_total: u64,
    pub has_selected_or_nominated_pair: bool,
    pub direct_checks_without_response: bool,
    pub local_candidate_type_summary: String,
    pub remote_candidate_type_summary: String,
    pub address_family_summary: String,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineRtcBuilderObservation {
    pub observation_id: u64,
    pub controlled_twcc_registry: bool,
    pub feedback_interval_ms: f64,
    pub registered_header_extensions: Vec<String>,
    pub registered_rtcp_feedback: Vec<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineTwccRemoteStreamObservation {
    pub observation_id: u64,
    pub ssrc: u32,
    pub mime_type: String,
    pub twcc_ext_id: Option<u8>,
    pub header_extensions: Vec<String>,
    pub rtcp_feedback: Vec<String>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineRemoteAnswerObservation {
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

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineTwccExtensionObservation {
    pub observation_id: u64,
    pub state: String,
    pub ssrc: u32,
    pub sequence_number: u16,
    pub expected_ext_id: u8,
    pub packet_seen_count: u64,
    pub missing_count: u64,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoTwccObservation {
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
    pub quality: XbxEngineTwccObservationQuality,
    pub delivery_ratio: f64,
    pub packet_loss_ratio: f64,
    pub observed_at_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XbxEngineTwccObservationQuality {
    Stable,
    Delayed,
    BootstrapSparse,
    RemoteObserved,
    Unstable,
}

impl XbxEngineTwccObservationQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Delayed => "delayed",
            Self::BootstrapSparse => "bootstrap-sparse",
            Self::RemoteObserved => "remote-observed",
            Self::Unstable => "unstable",
        }
    }
}

impl XbxEngineVideoTwccObservation {
    pub fn is_bootstrap_sparse_local_feedback(
        source: &str,
        feedback_interval_ms: Option<f64>,
        observed_packet_count: u16,
        covered_sequence_span: u16,
    ) -> bool {
        source == "local-feedback"
            && feedback_interval_ms.is_none()
            && observed_packet_count > 0
            && covered_sequence_span >= observed_packet_count.saturating_mul(3)
    }

    pub fn classify_quality(
        &self,
        expected_interval_ms: f64,
        stable_feedback_interval_ms: f64,
        stable_feedback_min_packets: u16,
    ) -> XbxEngineTwccObservationQuality {
        if self.source != "local-feedback" {
            return XbxEngineTwccObservationQuality::RemoteObserved;
        }
        if Self::is_bootstrap_sparse_local_feedback(
            &self.source,
            self.feedback_interval_ms,
            self.observed_packet_count,
            self.covered_sequence_span,
        ) {
            return XbxEngineTwccObservationQuality::BootstrapSparse;
        }
        if let Some(interval_ms) = self.feedback_interval_ms {
            if interval_ms >= expected_interval_ms.max(1.0) * 1.6 {
                return XbxEngineTwccObservationQuality::Delayed;
            }
        }
        let effective_stable_feedback_interval_ms =
            stable_feedback_interval_ms.max(expected_interval_ms.max(1.0) * 1.25) * 1.2;
        let stable_feedback = self.feedback_interval_ms.unwrap_or(0.0)
            <= effective_stable_feedback_interval_ms
            && self.observed_packet_count >= stable_feedback_min_packets
            && self.covered_sequence_span >= self.observed_packet_count;
        if stable_feedback {
            XbxEngineTwccObservationQuality::Stable
        } else {
            XbxEngineTwccObservationQuality::Unstable
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoTrackStatus {
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

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoRepairProbeObservation {
    pub observation_id: u64,
    pub phase: String,
    pub classification: String,
    pub stream_id: String,
    pub stream_ssrc: u32,
    pub mime_type: String,
    pub payload_type: u8,
    pub clock_rate: u32,
    pub associated_ssrc: Option<u32>,
    pub associated_payload_type: Option<u8>,
    pub stream_packet_count: u64,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoRtxReinjectObservation {
    pub stage: String,
    pub primary_ssrc: u32,
    pub repair_ssrc: u32,
    pub sequence_number: u16,
    pub rtp_timestamp: u32,
    pub pending_queue_len: usize,
    pub native_sequence_number: Option<u16>,
    pub matched_head_gap: bool,
    pub matched_nack_range: bool,
    pub matched_pending_gap: bool,
    pub matched_gap_sequence: Option<u16>,
    pub matched_nack_first_sequence: Option<u16>,
    pub matched_nack_last_sequence: Option<u16>,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineDataChannelMessageCatalogObservation {
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XbxEngineKeyframeRequestEpisodeObservation {
    pub episode_id: u64,
    pub request_reason: Option<String>,
    pub request_kind: Option<String>,
    pub status: String,
    pub status_detail: Option<String>,
    pub requested_at_ms: f64,
    pub sent_at_ms: Option<f64>,
    pub deadline_at_ms: Option<f64>,
    pub transport_detail: Option<String>,
    pub first_video_packet_at_ms: Option<f64>,
    pub first_video_packet_rtp_timestamp: Option<u32>,
    pub first_video_packet_is_keyframe: Option<bool>,
    pub first_keyframe_packet_at_ms: Option<f64>,
    pub first_keyframe_decoded_at_ms: Option<f64>,
    pub response_rtp_timestamp: Option<u32>,
    pub response_frame_seq: Option<u64>,
    pub response_verdict: Option<String>,
    /// 生命周期阶段：`requesting` / `sent` / `packetSeen` / `decoded` / `success` / `failure`
    pub lifecycle_phase: Option<String>,
    /// clean-anchor 成功后标记观测退场：默认可观测启发式匹配跳过；RTP 精确对齐时仍可绑定。
    pub retired_at_ms: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XbxEngineH264InspectionObservation {
    pub observation_id: u64,
    pub frame_rtp_timestamp: Option<u32>,
    pub nal_types: Vec<String>,
    pub nal_count: u16,
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
    pub sample_width: Option<u32>,
    pub sample_height: Option<u32>,
    pub bootstrap_ready: bool,
    pub bootstrap_reject_reason: Option<String>,
    pub continuation_verdict: Option<String>,
    pub admission_accepted: bool,
    pub observed_at_ms: f64,
    pub bound_episode_id: Option<u64>,
    pub bound_episode_status: Option<String>,
    pub bound_as_recovery_response: Option<bool>,
    pub bound_response_rtp_timestamp: Option<u32>,
    pub bound_recovery_epoch: Option<u64>,
    pub episode_phase_at_observation: Option<String>,
    pub is_post_recovery_degradation: Option<bool>,
    pub reject_classification: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XbxEnginePictureRecoveryTransitionObservation {
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XbxEnginePictureRecoveryBlockerObservation {
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XbxEngineVideoIngressTerminationObservation {
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XbxEngineFirstFrameLatencyObservation {
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
