use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use ohmygamepad_protocol::OhMyGamepadRumbleRequestDto;
use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
    XbxEngineSessionDto, XbxEngineTargetTypeDto, XbxEngineTransportStateDto, XbxEngineViewportDto,
};

use crate::api::input::{NoopXbxEngineInputBackend, XbxEngineInputBackend, XbxEngineInputStatus};
use crate::api::runtime::{XbxEngineRuntimeConfig, XbxEngineRuntimeError};
use crate::api::types;

pub use types::observations::*;

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineMediaNegotiationRequest {
    pub session: XbxEngineSessionDto,
    pub viewport: XbxEngineViewportDto,
    pub restart: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineMediaNegotiation {
    pub local_offer_sdp: String,
    pub local_candidates: Vec<XbxEngineIceCandidateDto>,
    pub surface_id: String,
    pub video_width: u32,
    pub video_height: u32,
    pub first_frame_packet_arrival_time_ms: Option<f64>,
    pub frame_decoded_time_ms: Option<f64>,
    pub frame_rendered_time_ms: Option<f64>,
    pub input_status: XbxEngineInputStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineVideoFrameStats {
    pub width: u32,
    pub height: u32,
    pub frame_seq: u64,
    pub fps: f64,
    pub rendered_at_ms: f64,
}

pub type CFDictionaryRef = *const std::ffi::c_void;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoColorMatrix {
    #[default]
    Bt709,
    Bt601,
    Smpte240M,
    Bt2020,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoColorPrimaries {
    #[default]
    Bt709,
    P3D65,
    Bt2020,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoTransferFunction {
    #[default]
    Bt709,
    Srgb,
    Linear,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoColorRange {
    #[default]
    Video,
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MacOsVideoChromaLocation {
    #[default]
    Center,
    Left,
    TopLeft,
    Unknown,
}

pub struct MacOsCVPixelBufferDescriptor {
    pub ptr: *mut std::ffi::c_void,
    pub color_matrix: MacOsVideoColorMatrix,
    pub color_primaries: MacOsVideoColorPrimaries,
    pub transfer_function: MacOsVideoTransferFunction,
    pub color_range: MacOsVideoColorRange,
    pub chroma_location: MacOsVideoChromaLocation,
    pub drop_fn: Option<Box<dyn FnOnce(*mut std::ffi::c_void) + Send + Sync>>,
}

impl std::fmt::Debug for MacOsCVPixelBufferDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacOsCVPixelBufferDescriptor")
            .field("ptr", &self.ptr)
            .field("color_matrix", &self.color_matrix)
            .field("color_primaries", &self.color_primaries)
            .field("transfer_function", &self.transfer_function)
            .field("color_range", &self.color_range)
            .field("chroma_location", &self.chroma_location)
            .field(
                "drop_fn",
                &if self.drop_fn.is_some() {
                    "Some(<closure>)"
                } else {
                    "None"
                },
            )
            .finish()
    }
}

impl Drop for MacOsCVPixelBufferDescriptor {
    fn drop(&mut self) {
        if let Some(drop_fn) = self.drop_fn.take() {
            drop_fn(self.ptr);
        }
    }
}

unsafe impl Send for MacOsCVPixelBufferDescriptor {}
unsafe impl Sync for MacOsCVPixelBufferDescriptor {}

pub struct WindowsD3d11TextureDescriptor {
    pub texture_ptr: *mut std::ffi::c_void,
    pub shared_handle: *mut std::ffi::c_void,
    pub dxgi_format: u32,
    pub array_slice: u32,
    pub color_matrix: MacOsVideoColorMatrix,
    pub color_primaries: MacOsVideoColorPrimaries,
    pub transfer_function: MacOsVideoTransferFunction,
    pub color_range: MacOsVideoColorRange,
    pub chroma_location: MacOsVideoChromaLocation,
    pub drop_fn:
        Option<Box<dyn FnOnce(*mut std::ffi::c_void, *mut std::ffi::c_void) + Send + Sync>>,
}

impl std::fmt::Debug for WindowsD3d11TextureDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsD3d11TextureDescriptor")
            .field("texture_ptr", &self.texture_ptr)
            .field("shared_handle", &self.shared_handle)
            .field("dxgi_format", &self.dxgi_format)
            .field("array_slice", &self.array_slice)
            .field("color_matrix", &self.color_matrix)
            .field("color_primaries", &self.color_primaries)
            .field("transfer_function", &self.transfer_function)
            .field("color_range", &self.color_range)
            .field("chroma_location", &self.chroma_location)
            .field(
                "drop_fn",
                &if self.drop_fn.is_some() {
                    "Some(<closure>)"
                } else {
                    "None"
                },
            )
            .finish()
    }
}

impl Drop for WindowsD3d11TextureDescriptor {
    fn drop(&mut self) {
        if let Some(drop_fn) = self.drop_fn.take() {
            drop_fn(self.texture_ptr, self.shared_handle);
        }
    }
}

unsafe impl Send for WindowsD3d11TextureDescriptor {}
unsafe impl Sync for WindowsD3d11TextureDescriptor {}

#[derive(Clone, Debug)]
pub enum XbxEngineRenderPixelData {
    Rgba {
        bytes: Arc<[u8]>,
    },
    Bgra {
        bytes: Arc<[u8]>,
    },
    Nv12 {
        y_plane: Arc<[u8]>,
        uv_plane: Arc<[u8]>,
        y_stride: u32,
        uv_stride: u32,
    },
    Descriptor {
        handle: Arc<dyn std::any::Any + Send + Sync>,
    },
}

impl PartialEq for XbxEngineRenderPixelData {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Rgba { bytes: l }, Self::Rgba { bytes: r }) => l == r,
            (Self::Bgra { bytes: l }, Self::Bgra { bytes: r }) => l == r,
            (
                Self::Nv12 {
                    y_plane: ly,
                    uv_plane: luv,
                    y_stride: lys,
                    uv_stride: luvs,
                },
                Self::Nv12 {
                    y_plane: ry,
                    uv_plane: ruv,
                    y_stride: rys,
                    uv_stride: ruvs,
                },
            ) => ly == ry && luv == ruv && lys == rys && luvs == ruvs,
            (Self::Descriptor { handle: l }, Self::Descriptor { handle: r }) => Arc::ptr_eq(l, r),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineRenderFrame {
    pub width: u32,
    pub height: u32,
    pub frame_seq: u64,
    pub rendered_at_ms: f64,
    pub rtp_timestamp: Option<u32>,
    pub recovery_epoch_tag: Option<u64>,
    pub recovery_owner_rtp_timestamp: Option<u32>,
    pub is_keyframe: bool,
    pub frame_recovery_disposition: Option<String>,
    pub frame_unrecoverable_reason: Option<String>,
    pub presentation_value_role: Option<String>,
    pub pixel_data: XbxEngineRenderPixelData,
}

/// Viewport / surface route mirrored from engine runtime for decode→host 推式投递（renderer 线程读取）。
#[derive(Clone, Debug, Default)]
pub struct XbxEngineHostPresentRoute {
    pub viewport: Option<XbxEngineViewportDto>,
    pub surface_id: Option<String>,
}

/// 在 renderer 接受帧后立即投递到宿主 mailbox，避免依赖 `runtime.tick` 周期性 pull。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XbxHostRenderFramePushOutcome {
    Accepted,
    RouteUnavailable,
    RegistryUnavailable,
    Rejected,
}

pub trait XbxHostRenderFramePush: Send + Sync {
    fn push_render_frame_for_host_present(
        &self,
        frame: XbxEngineRenderFrame,
    ) -> XbxHostRenderFramePushOutcome;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XbxEnginePresentationValueRole {
    FreshAnchor,
    RecoveryContinuation,
    SteadyContinuation,
    Disposable,
}

impl XbxEnginePresentationValueRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FreshAnchor => "fresh_anchor",
            Self::RecoveryContinuation => "recovery_continuation",
            Self::SteadyContinuation => "steady_continuation",
            Self::Disposable => "disposable",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::FreshAnchor => 3,
            Self::RecoveryContinuation => 2,
            Self::SteadyContinuation => 1,
            Self::Disposable => 0,
        }
    }

    pub fn protects_anchor(self) -> bool {
        matches!(self, Self::FreshAnchor | Self::RecoveryContinuation)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XbxEngineLatestOnlyFrameMeta {
    pub presentation_value_role: XbxEnginePresentationValueRole,
    pub recovery_epoch_tag: Option<u64>,
    pub recovery_owner_rtp_timestamp: Option<u32>,
    pub rtp_timestamp: Option<u32>,
    pub frame_seq: Option<u64>,
    pub rendered_at_ms: f64,
    pub owner_preference_active: bool,
    pub value_rank: u8,
}

impl XbxEngineLatestOnlyFrameMeta {
    pub fn owner_matches_frame(self) -> Option<bool> {
        self.recovery_owner_rtp_timestamp
            .zip(self.rtp_timestamp)
            .map(|(owner_rtp, frame_rtp)| owner_rtp == frame_rtp)
    }
}

fn latest_only_frame_value_rank(
    recovery_disposition: Option<&str>,
    unrecoverable_reason: Option<&str>,
) -> u8 {
    if unrecoverable_reason.is_some() {
        0
    } else if matches!(
        recovery_disposition,
        Some("rebuilding" | "rebuilding-supply")
    ) {
        3
    } else if matches!(recovery_disposition, Some("repairing")) {
        1
    } else {
        2
    }
}

fn infer_presentation_value_role(
    recovery_disposition: Option<&str>,
    unrecoverable_reason: Option<&str>,
) -> XbxEnginePresentationValueRole {
    if unrecoverable_reason.is_some() {
        XbxEnginePresentationValueRole::Disposable
    } else if matches!(
        recovery_disposition,
        Some("rebuilding" | "rebuilding-supply")
    ) {
        XbxEnginePresentationValueRole::FreshAnchor
    } else if matches!(recovery_disposition, Some("repairing")) {
        XbxEnginePresentationValueRole::RecoveryContinuation
    } else {
        XbxEnginePresentationValueRole::SteadyContinuation
    }
}

pub fn compare_latest_only_frame_meta(
    existing: &XbxEngineLatestOnlyFrameMeta,
    incoming: &XbxEngineLatestOnlyFrameMeta,
) -> i32 {
    match existing
        .presentation_value_role
        .rank()
        .cmp(&incoming.presentation_value_role.rank())
    {
        std::cmp::Ordering::Greater => return 1,
        std::cmp::Ordering::Less => return -1,
        std::cmp::Ordering::Equal => {}
    }

    match (existing.recovery_epoch_tag, incoming.recovery_epoch_tag) {
        (Some(existing_epoch), Some(incoming_epoch)) => match existing_epoch.cmp(&incoming_epoch) {
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Equal => {}
        },
        (Some(_), None) => return 1,
        (None, Some(_)) => return -1,
        (None, None) => {}
    }

    if existing.recovery_epoch_tag == incoming.recovery_epoch_tag {
        match (
            existing.recovery_owner_rtp_timestamp,
            incoming.recovery_owner_rtp_timestamp,
        ) {
            (Some(existing_owner), Some(incoming_owner)) => {
                match existing_owner.cmp(&incoming_owner) {
                    std::cmp::Ordering::Greater => return 1,
                    std::cmp::Ordering::Less => return -1,
                    std::cmp::Ordering::Equal => {}
                }
            }
            (Some(_), None) => {
                if existing.presentation_value_role.protects_anchor() {
                    return 1;
                }
            }
            (None, Some(_)) => {
                if incoming.presentation_value_role.protects_anchor() {
                    return -1;
                }
            }
            (None, None) => {}
        }
    }

    if existing.owner_preference_active || incoming.owner_preference_active {
        let existing_matches_owner = existing.owner_matches_frame() == Some(true);
        let incoming_matches_owner = incoming.owner_matches_frame() == Some(true);
        match existing_matches_owner.cmp(&incoming_matches_owner) {
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Equal => {}
        }
    }

    match existing.value_rank.cmp(&incoming.value_rank) {
        std::cmp::Ordering::Greater => return 1,
        std::cmp::Ordering::Less => return -1,
        std::cmp::Ordering::Equal => {}
    }

    match (existing.rtp_timestamp, incoming.rtp_timestamp) {
        (Some(existing_rtp), Some(incoming_rtp)) => match existing_rtp.cmp(&incoming_rtp) {
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Equal => {}
        },
        (Some(_), None) => return 1,
        (None, Some(_)) => return -1,
        (None, None) => {}
    }

    match (existing.frame_seq, incoming.frame_seq) {
        (Some(existing_seq), Some(incoming_seq)) => match existing_seq.cmp(&incoming_seq) {
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Equal => {}
        },
        (Some(_), None) => return 1,
        (None, Some(_)) => return -1,
        (None, None) => {}
    }

    if existing.rendered_at_ms > incoming.rendered_at_ms {
        1
    } else if existing.rendered_at_ms < incoming.rendered_at_ms {
        -1
    } else {
        0
    }
}

impl XbxEngineRenderFrame {
    pub fn latest_only_frame_meta(&self) -> XbxEngineLatestOnlyFrameMeta {
        let value_rank = latest_only_frame_value_rank(
            self.frame_recovery_disposition.as_deref(),
            self.frame_unrecoverable_reason.as_deref(),
        );
        XbxEngineLatestOnlyFrameMeta {
            presentation_value_role: self
                .presentation_value_role
                .as_deref()
                .map(presentation_value_role_from_label)
                .unwrap_or_else(|| {
                    infer_presentation_value_role(
                        self.frame_recovery_disposition.as_deref(),
                        self.frame_unrecoverable_reason.as_deref(),
                    )
                }),
            recovery_epoch_tag: self.recovery_epoch_tag,
            recovery_owner_rtp_timestamp: self.recovery_owner_rtp_timestamp,
            rtp_timestamp: self.rtp_timestamp,
            frame_seq: Some(self.frame_seq),
            rendered_at_ms: self.rendered_at_ms,
            owner_preference_active: value_rank == 1 || value_rank == 3,
            value_rank,
        }
    }
}

pub fn presentation_value_role_from_label(label: &str) -> XbxEnginePresentationValueRole {
    match label {
        "fresh_anchor" => XbxEnginePresentationValueRole::FreshAnchor,
        "recovery_continuation" => XbxEnginePresentationValueRole::RecoveryContinuation,
        "steady_continuation" => XbxEnginePresentationValueRole::SteadyContinuation,
        _ => XbxEnginePresentationValueRole::Disposable,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct XbxEngineMediaRuntimeStats {
    pub transport_state: XbxEngineTransportStateDto,
    pub session_target_type: Option<XbxEngineTargetTypeDto>,
    pub session_phase: Option<String>,
    pub message_handshake_acked_at_ms: Option<f64>,
    pub control_ready_at_ms: Option<f64>,
    pub control_pending_replay_action_count: u8,
    pub control_pending_replay_since_ms: Option<f64>,
    pub control_pending_replay_summary: Option<String>,
    pub transport_policy_profile: Option<String>,
    pub recovery_policy_profile: Option<String>,
    pub recovery_diagnosis: Option<String>,
    /// 与当拍 `RecoveryPolicyProposal.decision.action` 同源的 RFC cost tier；投影到 `XbxEngineStatsDto.recovery_rfc_ceiling`。
    pub recovery_rfc_authoritative_ceiling: Option<String>,
    /// 与当拍 `RecoveryPolicyProposal.reason` 同源的 RFC 故障域（`SessionFaultDomain::as_rfc_str()`）。
    pub recovery_rfc_authoritative_fault_domain: Option<String>,
    /// 与当拍 `VideoSchedulingOwnerState` 映射的 RFC 阶段（`SessionRecoveryStage::as_rfc_str()`）。
    pub recovery_rfc_authoritative_stage: Option<String>,
    /// 当拍 `RecoveryPolicyProposal.reason` 的稳定标签（`VideoEscalationReason::label()`）；供控制面替代 `recovery_diagnosis`。
    pub recovery_active_escalation_reason: Option<String>,
    /// RFC 2026-05-13：owner 表面状态（仅 trace/diagnostics）。
    pub recovery_owner_surface_state: Option<String>,
    /// RFC 2026-05-13：锚点证据摘要。
    pub recovery_anchor_evidence: Option<String>,
    /// RFC 2026-05-13：升级依据（local_supply / anchor_missing / connectivity_bad）。
    pub recovery_escalation_basis: Option<String>,
    /// RFC 2026-05-14：恢复时序用的平滑 RTT（上升快、下降慢）。
    pub recovery_smoothed_rtt_ms: Option<f64>,
    /// RFC 2026-05-14：当拍解析用的有效 RTT（与 diagnostics 对齐）。
    pub recovery_effective_rtt_ms: Option<f64>,
    pub recovery_dynamic_nack_timeout_ms: Option<f64>,
    pub recovery_dynamic_nack_retry_interval_ms: Option<f64>,
    pub recovery_dynamic_pli_refresh_interval_ms: Option<f64>,
    pub recovery_dynamic_fir_retry_interval_ms: Option<f64>,
    pub recovery_dynamic_decoded_pending_commit_hold_ms: Option<f64>,
    pub recovery_dynamic_continuation_patience_ms: Option<f64>,
    pub recovery_dynamic_clean_anchor_patience_ms: Option<f64>,
    /// RFC 2026-05-14：H264 bootstrap SPS/PPS salvage 是否已应用到当拍 AU。
    pub recovery_codec_bootstrap_salvage_applied: Option<bool>,
    pub recovery_codec_bootstrap_salvage_failed_reason: Option<String>,
    pub recovery_nack_first_attempt_survival_window_ms: Option<f64>,
    pub recovery_nack_first_attempt_deadline_at_ms: Option<f64>,
    pub recovery_nack_first_attempt_still_economical: Option<bool>,
    pub recovery_nack_retry_allowed_reason: Option<String>,
    pub recovery_nack_retry_suppressed_reason: Option<String>,
    pub recovery_phase: Option<String>,
    pub recovery_exit_gate: Option<String>,
    pub recovery_ingress_waiting: Option<bool>,
    pub recovery_transport_await_unresolved: Option<bool>,
    pub recovery_playback_recovered_at_ms: Option<f64>,
    pub recovery_playback_recovered_phase: Option<String>,
    pub recovery_fresh_anchor_recovered_at_ms: Option<f64>,
    /// Host 已显示 bootstrap IDR 的 RTP（控制面 fresh-anchor 唯一事实源之一）。
    pub recovery_displayed_idr_rtp: Option<u32>,
    pub recovery_displayed_idr_at_ms: Option<f64>,
    /// Decode 已产出 config-change IDR、待 host 显示的 RTP 提示。
    pub recovery_pending_displayed_idr_rtp: Option<u32>,
    pub recovery_hard_fallback_timer_ms: Option<f64>,
    pub recovery_hard_fallback_trigger_reason: Option<String>,
    pub recovery_hard_fallback_timer_reset_reason: Option<String>,
    pub baseline_remote_profile: Option<String>,
    pub dynamic_remote_subprofile: Option<String>,
    pub effective_remote_profile_label: Option<String>,
    pub video_owner_state: Option<String>,
    /// 对外四态合同：starting / playing / waitingKeyframe / displayStalled。
    pub video_owner_contract_state: Option<String>,
    pub video_owner_reason: Option<String>,
    pub video_owner_source: Option<String>,
    pub video_owner_observed_at_ms: Option<f64>,
    pub transport_recovery_episode_active: bool,
    pub transport_recovery_episode_opened_at_ms: Option<f64>,
    pub transport_recovery_episode_closed_at_ms: Option<f64>,
    pub transport_recovery_episode_close_reason: Option<String>,
    pub video_anchor_clean_epoch: Option<u64>,
    pub video_anchor_clean_observed_at_ms: Option<f64>,
    pub video_anchor_clean_source_event: Option<String>,
    pub video_anchor_bridge_epoch: Option<u64>,
    pub video_anchor_bridge_observed_at_ms: Option<f64>,
    pub video_anchor_bridge_source_event: Option<String>,
    pub video_anchor_bridge_rtp_timestamp: Option<u32>,
    pub latest_clean_anchor_submission_epoch: Option<u64>,
    pub latest_clean_anchor_submission_episode_id: Option<u64>,
    pub latest_clean_anchor_submission_rtp_timestamp: Option<u32>,
    pub latest_clean_anchor_submission_observed_at_ms: Option<f64>,
    pub latest_clean_anchor_submission_source_event: Option<String>,
    pub direct_gaming_bitrate_band: Option<String>,
    pub latest_video_frame: Option<XbxEngineVideoFrameStats>,
    pub latest_observation_label: Option<String>,
    pub latest_observation_summary: Option<String>,
    pub latest_feedback_target_availability_state: Option<String>,
    pub latest_feedback_target_availability_reason: Option<String>,
    pub latest_feedback_target_availability_target: Option<String>,
    pub latest_feedback_target_availability_observed_at_ms: Option<f64>,
    pub latest_video_rtcp_send_failure_time_ms: Option<f64>,
    pub latest_video_rtcp_send_failure_reason: Option<String>,
    pub latest_keyframe_request_episode: Option<XbxEngineKeyframeRequestEpisodeObservation>,
    pub latest_h264_inspection_observation: Option<XbxEngineH264InspectionObservation>,
    pub latest_picture_recovery_transition_observation:
        Option<XbxEnginePictureRecoveryTransitionObservation>,
    pub latest_picture_recovery_blocker_observation:
        Option<XbxEnginePictureRecoveryBlockerObservation>,
    pub latest_video_ingress_termination_observation:
        Option<XbxEngineVideoIngressTerminationObservation>,
    pub latest_first_frame_latency_observation: Option<XbxEngineFirstFrameLatencyObservation>,
    pub latest_target_remb_action: Option<String>,
    pub latest_target_remb_summary: Option<String>,
    pub latest_video_stream_width: Option<u32>,
    pub latest_video_stream_height: Option<u32>,
    pub first_video_packet_arrival_time_ms: Option<f64>,
    pub latest_video_packet_arrival_time_ms: Option<f64>,
    pub latest_video_packet_arrival_rtp_timestamp: Option<u32>,
    pub first_audio_packet_arrival_time_ms: Option<f64>,
    pub latest_audio_packet_arrival_time_ms: Option<f64>,
    pub latest_audio_playout_time_ms: Option<f64>,
    pub audio_playout_latency_ms: Option<f64>,
    pub inbound_video_frame_rate_fps: f64,
    pub latest_video_packet_sequence: Option<u16>,
    pub latest_video_packet_gap: Option<XbxEngineVideoPacketGapObservation>,
    pub inbound_video_packet_count_total: u64,
    pub inbound_video_packet_loss_estimate_total: u64,
    pub inbound_video_loss_ratio_1s: f64,
    pub inbound_video_loss_ratio_5s: f64,
    pub inbound_video_jitter_ms: Option<f64>,
    pub video_nack_request_count_total: u64,
    pub video_nack_batch_count_total: u64,
    pub video_nack_per_sec: f64,
    pub latest_video_nack_observation: Option<XbxEngineVideoNackObservation>,
    pub latest_video_escalation_observation: Option<XbxEngineVideoEscalationObservation>,
    pub latest_recovery_decision_ledger: Option<XbxEngineRecoveryDecisionLedgerObservation>,
    pub recent_recovery_decision_ledgers: Vec<XbxEngineRecoveryDecisionLedgerObservation>,
    pub latest_video_timeline_observation: Option<XbxEngineVideoTimelineObservation>,
    pub latest_video_receiver_observation: Option<XbxEngineVideoReceiverObservation>,
    pub latest_anchor_candidate_ledger: Option<XbxEngineAnchorCandidateLedger>,
    pub latest_video_bwe_observation: Option<XbxEngineVideoBweObservation>,
    pub latest_video_twcc_observation: Option<XbxEngineVideoTwccObservation>,
    pub latest_rtc_builder_observation: Option<XbxEngineRtcBuilderObservation>,
    pub latest_twcc_remote_stream_observation: Option<XbxEngineTwccRemoteStreamObservation>,
    pub latest_remote_answer_observation: Option<XbxEngineRemoteAnswerObservation>,
    pub latest_twcc_extension_observation: Option<XbxEngineTwccExtensionObservation>,
    pub latest_video_track_status: Option<XbxEngineVideoTrackStatus>,
    pub latest_video_repair_probe_observation: Option<XbxEngineVideoRepairProbeObservation>,
    pub latest_video_rtx_reinject_observation: Option<XbxEngineVideoRtxReinjectObservation>,
    pub latest_data_channel_message_catalog_observation:
        Option<XbxEngineDataChannelMessageCatalogObservation>,
    pub recent_keyframe_request_episodes: Vec<XbxEngineKeyframeRequestEpisodeObservation>,
    /// 连续「已发出且终端失败」的 keyframe 请求次数（用于 decoder reset 门槛；clean anchor / 新 recovery epoch 清零）。
    pub keyframe_consecutive_sent_failures: u8,
    /// 已为 `keyframe_consecutive_sent_failures` 计数过的 episode，避免重复累加。
    pub keyframe_sent_failure_last_counted_episode_id: Option<u64>,
    pub transport_recovery_epoch: u64,
    pub transport_recovery_epoch_at_last_escalation: u64,
    pub picture_recovery_transition_observation_count: u64,
    pub picture_recovery_blocker_observation_count: u64,
    pub video_ingress_termination_observation_count: u64,
    pub first_frame_latency_observation_count: u64,
    pub video_ingress_termination_id_seq: u64,
    pub latest_video_ingress_termination_id: Option<u64>,
    pub latest_video_ingress_close_intent_cause: Option<String>,
    pub latest_video_ingress_close_intent_observed_at_ms: Option<f64>,
    pub video_repair_probe_stream_bind_count_total: u64,
    pub video_repair_probe_packet_count_total: u64,
    pub video_repair_probe_active_since_ms: Option<f64>,
    pub video_repair_probe_recovered_count_since_active: u64,
    pub video_repair_probe_late_recovered_count_since_active: u64,
    pub video_repair_probe_expired_count_since_active: u64,
    pub video_repair_probe_packet_gap_count_since_active: u64,
    pub video_repair_probe_recovery_hit_rate_since_active: Option<f64>,
    pub video_rtx_reinject_head_match_count_total: u64,
    pub video_rtx_reinject_range_match_count_total: u64,
    pub video_rtx_reinject_miss_count_total: u64,
    pub video_pli_request_count_total: u64,
    pub video_pli_per_min: f64,
    pub video_pending_missing_packets: usize,
    pub video_loss_finalized_count_total: u64,
    pub video_loss_recovered_count_total: u64,
    pub video_loss_late_recovered_count_total: u64,
    pub video_nack_recovery_rtt_ms: Option<f64>,
    pub video_rtt_ms: Option<f64>,
    pub video_rtt_source: Option<String>,
    pub video_remb_bps: Option<u32>,
    pub inbound_bitrate_kbps: Option<f64>,
    pub inbound_video_bitrate_kbps: Option<f64>,
    pub inbound_audio_bitrate_kbps: Option<f64>,
    pub actual_video_bitrate_source: Option<String>,
    pub transport_path: Option<String>,
    pub transport_candidate_pair: Option<String>,
    pub transport_protocol: Option<String>,
    pub transport_address_family: Option<String>,
    pub latest_video_decode_ok_time_ms: Option<f64>,
    pub latest_video_decode_ok_rtp_timestamp: Option<u32>,
    pub video_decode_fps: f64,
    pub video_decoder_stalled: Option<bool>,
    pub video_decoder_backend_name: Option<String>,
    pub latest_video_decoder_probe_observation: Option<XbxEngineVideoDecoderProbeObservation>,
    pub latest_video_decoder_bootstrap_gate_observation:
        Option<XbxEngineVideoDecoderBootstrapGateObservation>,
    pub latest_decode_output_path_observation: Option<XbxEngineDecodeOutputPathObservation>,
    pub latest_remote_frame_capture_observation: Option<XbxEngineRemoteFrameCaptureObservation>,
    pub video_decoder_hardware_failure_streak: u32,
    pub latest_video_decoder_hardware_failure_time_ms: Option<f64>,
    pub latest_video_decoder_hardware_failure_status: Option<i32>,
    pub video_decoder_recovery_state: Option<String>,
    pub video_decoder_recovery_event: Option<String>,
    pub video_decoder_recovery_detail: Option<String>,
    pub video_decoder_recovery_status: Option<i32>,
    pub video_decoder_recovery_state_changed_at_ms: Option<f64>,
    pub video_decoder_reset_count: u64,
    pub latest_video_decoder_reset_time_ms: Option<f64>,
    pub video_decode_input_drop_count_total: u64,
    pub video_decode_output_drop_count_total: u64,
    pub video_pacer_submit_count_total: u64,
    pub video_pacer_drop_count_total: u64,
    pub video_renderer_submit_count_total: u64,
    pub video_renderer_drop_count_total: u64,
    pub host_mailbox_drop_count_total: u64,
    pub host_mailbox_overwrite_count_total: u64,
    pub host_mailbox_enqueue_count_total: u64,
    pub host_no_pending_take_count_total: u64,
    pub host_no_pending_streak: u32,
    pub host_no_pending_max_streak: u32,
    pub host_no_pending_pressure_level: Option<String>,
    pub host_mailbox_submit_epoch: u64,
    pub host_display_tick_epoch: u64,
    pub host_frame_present_epoch: u64,
    /// 由 session policy 写入：host present 停滞时仅允许关键帧进入解码。
    pub host_present_stall_decode_throttle: bool,
    pub host_cadence_phase: Option<String>,
    pub video_present_descriptor_upload_mode: Option<String>,
    pub video_present_descriptor_metal_import_count_total: u64,
    pub video_present_descriptor_cpu_upload_count_total: u64,
    pub host_display_interval_ms: Option<f64>,
    pub host_frame_age_budget_ms: Option<f64>,
    pub latest_host_mailbox_submit_time_ms: Option<f64>,
    pub latest_video_host_submit_rtp_timestamp: Option<u32>,
    pub latest_video_host_present_time_ms: Option<f64>,
    pub host_view_generation: u64,
    pub latest_host_view_created_at_ms: Option<f64>,
    pub submit_age_ms: Option<f64>,
    pub display_age_ms: Option<f64>,
    pub last_displayed_frame_seq: Option<u64>,
    pub last_displayed_frame_rtp_timestamp: Option<u32>,
    pub last_displayed_at_ms: Option<f64>,
    pub video_present_fps: f64,
    pub video_renderer_stalled: Option<bool>,
    pub latest_decode_candidate_decision: Option<XbxEnginePipelineCandidateDecisionObservation>,
    pub latest_render_mailbox_decision: Option<XbxEnginePipelineCandidateDecisionObservation>,
    pub latest_video_frame_drop: Option<XbxEngineVideoFrameDropObservation>,
    pub latest_video_frame_recovery_observation: Option<XbxEngineFrameRecoveryObservation>,
    pub inbound_bytes_total: u64,
    pub inbound_video_bytes_total: u64,
    pub inbound_primary_video_bytes_total: u64,
    pub inbound_audio_bytes_total: u64,
}

impl Default for XbxEngineMediaRuntimeStats {
    fn default() -> Self {
        Self {
            transport_state: XbxEngineTransportStateDto::New,
            session_target_type: None,
            session_phase: None,
            message_handshake_acked_at_ms: None,
            control_ready_at_ms: None,
            control_pending_replay_action_count: 0,
            control_pending_replay_since_ms: None,
            control_pending_replay_summary: None,
            transport_policy_profile: None,
            recovery_policy_profile: None,
            recovery_diagnosis: None,
            recovery_rfc_authoritative_ceiling: None,
            recovery_rfc_authoritative_fault_domain: None,
            recovery_rfc_authoritative_stage: None,
            recovery_active_escalation_reason: None,
            recovery_owner_surface_state: None,
            recovery_anchor_evidence: None,
            recovery_escalation_basis: None,
            recovery_smoothed_rtt_ms: None,
            recovery_effective_rtt_ms: None,
            recovery_dynamic_nack_timeout_ms: None,
            recovery_dynamic_nack_retry_interval_ms: None,
            recovery_dynamic_pli_refresh_interval_ms: None,
            recovery_dynamic_fir_retry_interval_ms: None,
            recovery_dynamic_decoded_pending_commit_hold_ms: None,
            recovery_dynamic_continuation_patience_ms: None,
            recovery_dynamic_clean_anchor_patience_ms: None,
            recovery_codec_bootstrap_salvage_applied: None,
            recovery_codec_bootstrap_salvage_failed_reason: None,
            recovery_nack_first_attempt_survival_window_ms: None,
            recovery_nack_first_attempt_deadline_at_ms: None,
            recovery_nack_first_attempt_still_economical: None,
            recovery_nack_retry_allowed_reason: None,
            recovery_nack_retry_suppressed_reason: None,
            recovery_phase: None,
            recovery_exit_gate: None,
            recovery_ingress_waiting: None,
            recovery_transport_await_unresolved: None,
            recovery_playback_recovered_at_ms: None,
            recovery_playback_recovered_phase: None,
            recovery_fresh_anchor_recovered_at_ms: None,
            recovery_displayed_idr_rtp: None,
            recovery_displayed_idr_at_ms: None,
            recovery_pending_displayed_idr_rtp: None,
            recovery_hard_fallback_timer_ms: None,
            recovery_hard_fallback_trigger_reason: None,
            recovery_hard_fallback_timer_reset_reason: None,
            baseline_remote_profile: None,
            dynamic_remote_subprofile: None,
            effective_remote_profile_label: None,
            video_owner_state: None,
            video_owner_contract_state: None,
            video_owner_reason: None,
            video_owner_source: None,
            video_owner_observed_at_ms: None,
            transport_recovery_episode_active: false,
            transport_recovery_episode_opened_at_ms: None,
            transport_recovery_episode_closed_at_ms: None,
            transport_recovery_episode_close_reason: None,
            video_anchor_clean_epoch: None,
            video_anchor_clean_observed_at_ms: None,
            video_anchor_clean_source_event: None,
            video_anchor_bridge_epoch: None,
            video_anchor_bridge_observed_at_ms: None,
            video_anchor_bridge_source_event: None,
            video_anchor_bridge_rtp_timestamp: None,
            latest_clean_anchor_submission_epoch: None,
            latest_clean_anchor_submission_episode_id: None,
            latest_clean_anchor_submission_rtp_timestamp: None,
            latest_clean_anchor_submission_observed_at_ms: None,
            latest_clean_anchor_submission_source_event: None,
            direct_gaming_bitrate_band: None,
            latest_video_frame: None,
            latest_observation_label: None,
            latest_observation_summary: None,
            latest_feedback_target_availability_state: None,
            latest_feedback_target_availability_reason: None,
            latest_feedback_target_availability_target: None,
            latest_feedback_target_availability_observed_at_ms: None,
            latest_video_rtcp_send_failure_time_ms: None,
            latest_video_rtcp_send_failure_reason: None,
            latest_keyframe_request_episode: None,
            latest_h264_inspection_observation: None,
            latest_picture_recovery_transition_observation: None,
            latest_picture_recovery_blocker_observation: None,
            latest_video_ingress_termination_observation: None,
            latest_first_frame_latency_observation: None,
            latest_target_remb_action: None,
            latest_target_remb_summary: None,
            latest_video_stream_width: None,
            latest_video_stream_height: None,
            first_video_packet_arrival_time_ms: None,
            latest_video_packet_arrival_time_ms: None,
            latest_video_packet_arrival_rtp_timestamp: None,
            first_audio_packet_arrival_time_ms: None,
            latest_audio_packet_arrival_time_ms: None,
            latest_audio_playout_time_ms: None,
            audio_playout_latency_ms: None,
            inbound_video_frame_rate_fps: 0.0,
            latest_video_packet_sequence: None,
            latest_video_packet_gap: None,
            inbound_video_packet_count_total: 0,
            inbound_video_packet_loss_estimate_total: 0,
            inbound_video_loss_ratio_1s: 0.0,
            inbound_video_loss_ratio_5s: 0.0,
            inbound_video_jitter_ms: None,
            video_nack_request_count_total: 0,
            video_nack_batch_count_total: 0,
            video_nack_per_sec: 0.0,
            latest_video_nack_observation: None,
            latest_video_escalation_observation: None,
            latest_recovery_decision_ledger: None,
            recent_recovery_decision_ledgers: Vec::new(),
            latest_video_timeline_observation: None,
            latest_video_receiver_observation: None,
            latest_anchor_candidate_ledger: None,
            latest_video_bwe_observation: None,
            latest_video_twcc_observation: None,
            latest_rtc_builder_observation: None,
            latest_twcc_remote_stream_observation: None,
            latest_remote_answer_observation: None,
            latest_twcc_extension_observation: None,
            latest_video_track_status: None,
            latest_video_repair_probe_observation: None,
            latest_video_rtx_reinject_observation: None,
            latest_data_channel_message_catalog_observation: None,
            recent_keyframe_request_episodes: Vec::new(),
            keyframe_consecutive_sent_failures: 0,
            keyframe_sent_failure_last_counted_episode_id: None,
            transport_recovery_epoch: 0,
            transport_recovery_epoch_at_last_escalation: 0,
            picture_recovery_transition_observation_count: 0,
            picture_recovery_blocker_observation_count: 0,
            video_ingress_termination_observation_count: 0,
            first_frame_latency_observation_count: 0,
            video_ingress_termination_id_seq: 0,
            latest_video_ingress_termination_id: None,
            latest_video_ingress_close_intent_cause: None,
            latest_video_ingress_close_intent_observed_at_ms: None,
            video_repair_probe_stream_bind_count_total: 0,
            video_repair_probe_packet_count_total: 0,
            video_repair_probe_active_since_ms: None,
            video_repair_probe_recovered_count_since_active: 0,
            video_repair_probe_late_recovered_count_since_active: 0,
            video_repair_probe_expired_count_since_active: 0,
            video_repair_probe_packet_gap_count_since_active: 0,
            video_repair_probe_recovery_hit_rate_since_active: None,
            video_rtx_reinject_head_match_count_total: 0,
            video_rtx_reinject_range_match_count_total: 0,
            video_rtx_reinject_miss_count_total: 0,
            video_pli_request_count_total: 0,
            video_pli_per_min: 0.0,
            video_pending_missing_packets: 0,
            video_loss_finalized_count_total: 0,
            video_loss_recovered_count_total: 0,
            video_loss_late_recovered_count_total: 0,
            video_nack_recovery_rtt_ms: None,
            video_rtt_ms: None,
            video_rtt_source: None,
            video_remb_bps: None,
            inbound_bitrate_kbps: None,
            inbound_video_bitrate_kbps: None,
            inbound_audio_bitrate_kbps: None,
            actual_video_bitrate_source: None,
            transport_path: None,
            transport_candidate_pair: None,
            transport_protocol: None,
            transport_address_family: None,
            latest_video_decode_ok_time_ms: None,
            latest_video_decode_ok_rtp_timestamp: None,
            video_decode_fps: 0.0,
            video_decoder_stalled: None,
            video_decoder_backend_name: None,
            latest_video_decoder_probe_observation: None,
            latest_video_decoder_bootstrap_gate_observation: None,
            latest_decode_output_path_observation: None,
            latest_remote_frame_capture_observation: None,
            video_decoder_hardware_failure_streak: 0,
            latest_video_decoder_hardware_failure_time_ms: None,
            latest_video_decoder_hardware_failure_status: None,
            video_decoder_recovery_state: None,
            video_decoder_recovery_event: None,
            video_decoder_recovery_detail: None,
            video_decoder_recovery_status: None,
            video_decoder_recovery_state_changed_at_ms: None,
            video_decoder_reset_count: 0,
            latest_video_decoder_reset_time_ms: None,
            video_decode_input_drop_count_total: 0,
            video_decode_output_drop_count_total: 0,
            video_pacer_submit_count_total: 0,
            video_pacer_drop_count_total: 0,
            video_renderer_submit_count_total: 0,
            video_renderer_drop_count_total: 0,
            host_mailbox_drop_count_total: 0,
            host_mailbox_overwrite_count_total: 0,
            host_mailbox_enqueue_count_total: 0,
            host_no_pending_take_count_total: 0,
            host_no_pending_streak: 0,
            host_no_pending_max_streak: 0,
            host_no_pending_pressure_level: None,
            host_display_tick_epoch: 0,
            host_mailbox_submit_epoch: 0,
            host_frame_present_epoch: 0,
            host_present_stall_decode_throttle: false,
            host_cadence_phase: None,
            video_present_descriptor_upload_mode: None,
            video_present_descriptor_metal_import_count_total: 0,
            video_present_descriptor_cpu_upload_count_total: 0,
            host_display_interval_ms: None,
            host_frame_age_budget_ms: None,
            latest_host_mailbox_submit_time_ms: None,
            latest_video_host_submit_rtp_timestamp: None,
            latest_video_host_present_time_ms: None,
            host_view_generation: 0,
            latest_host_view_created_at_ms: None,
            submit_age_ms: None,
            display_age_ms: None,
            last_displayed_frame_seq: None,
            last_displayed_frame_rtp_timestamp: None,
            last_displayed_at_ms: None,
            video_present_fps: 0.0,
            video_renderer_stalled: None,
            latest_decode_candidate_decision: None,
            latest_render_mailbox_decision: None,
            latest_video_frame_drop: None,
            latest_video_frame_recovery_observation: None,
            inbound_bytes_total: 0,
            inbound_video_bytes_total: 0,
            inbound_primary_video_bytes_total: 0,
            inbound_audio_bytes_total: 0,
        }
    }
}

pub trait XbxEngineMediaBackend: Send {
    fn sync_runtime_config(
        &mut self,
        _runtime_config: &XbxEngineRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn negotiate(
        &mut self,
        request: XbxEngineMediaNegotiationRequest,
    ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError>;
    fn create_offer(&mut self) -> Result<String, XbxEngineRuntimeError>;
    fn apply_remote_description(
        &mut self,
        answer_sdp: String,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn add_remote_ice_candidates(
        &mut self,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn local_candidates_snapshot(
        &self,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError>;
    fn local_ice_gathering_complete(&self) -> Result<bool, XbxEngineRuntimeError>;
    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn set_audio_volume(&mut self, value: f32) -> Result<(), XbxEngineRuntimeError>;
    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError>;
    fn press_controller_button(
        &mut self,
        button: String,
        duration_ms: u64,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError>;
    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn current_input_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError>;
    fn snapshot_runtime_stats(&self) -> Result<XbxEngineMediaRuntimeStats, XbxEngineRuntimeError>;
    fn take_pending_gamepad_rumble_requests(
        &mut self,
    ) -> Result<Vec<OhMyGamepadRumbleRequestDto>, XbxEngineRuntimeError> {
        Ok(Vec::new())
    }
    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Result<Option<XbxEnginePendingRuntimeRecoveryAction>, XbxEngineRuntimeError> {
        Ok(None)
    }
    fn update_host_video_timing(
        &mut self,
        _host_display_interval_ms: Option<f64>,
        _host_frame_age_budget_ms: Option<f64>,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
    fn update_host_video_present_metrics(
        &mut self,
        _metrics: XbxEngineHostVideoPresentMetrics,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
    fn record_host_video_frame_drop(
        &mut self,
        _event: XbxEngineHostVideoFrameDropEvent,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
    fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<XbxEngineRenderFrame>, XbxEngineRuntimeError>;
    fn record_video_frame_drop(
        &mut self,
        _observation: XbxEngineVideoFrameDropObservation,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError>;
}

impl<TMediaBackend> XbxEngineMediaBackend for Box<TMediaBackend>
where
    TMediaBackend: XbxEngineMediaBackend + ?Sized,
{
    fn sync_runtime_config(
        &mut self,
        runtime_config: &XbxEngineRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().sync_runtime_config(runtime_config)
    }

    fn negotiate(
        &mut self,
        request: XbxEngineMediaNegotiationRequest,
    ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError> {
        self.as_mut().negotiate(request)
    }

    fn create_offer(&mut self) -> Result<String, XbxEngineRuntimeError> {
        self.as_mut().create_offer()
    }

    fn apply_remote_description(
        &mut self,
        answer_sdp: String,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut()
            .apply_remote_description(answer_sdp, remote_candidates)
    }

    fn add_remote_ice_candidates(
        &mut self,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().add_remote_ice_candidates(remote_candidates)
    }

    fn local_candidates_snapshot(
        &self,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        self.as_ref().local_candidates_snapshot()
    }

    fn local_ice_gathering_complete(&self) -> Result<bool, XbxEngineRuntimeError> {
        self.as_ref().local_ice_gathering_complete()
    }

    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().apply_display_state(state)
    }

    fn set_audio_volume(&mut self, value: f32) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().set_audio_volume(value)
    }

    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().set_microphone_capturing(capturing)
    }

    fn press_controller_button(
        &mut self,
        button: String,
        duration_ms: u64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().press_controller_button(button, duration_ms)
    }

    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().set_keyboard_pointer_enabled(enabled)
    }

    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().push_keyboard_pointer_input(event)
    }

    fn current_input_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        self.as_ref().current_input_status()
    }

    fn snapshot_runtime_stats(&self) -> Result<XbxEngineMediaRuntimeStats, XbxEngineRuntimeError> {
        self.as_ref().snapshot_runtime_stats()
    }

    fn take_pending_gamepad_rumble_requests(
        &mut self,
    ) -> Result<Vec<OhMyGamepadRumbleRequestDto>, XbxEngineRuntimeError> {
        self.as_mut().take_pending_gamepad_rumble_requests()
    }

    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Result<Option<XbxEnginePendingRuntimeRecoveryAction>, XbxEngineRuntimeError> {
        self.as_mut().take_pending_runtime_recovery_action()
    }

    fn update_host_video_timing(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut()
            .update_host_video_timing(host_display_interval_ms, host_frame_age_budget_ms)
    }

    fn update_host_video_present_metrics(
        &mut self,
        metrics: XbxEngineHostVideoPresentMetrics,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().update_host_video_present_metrics(metrics)
    }

    fn record_host_video_frame_drop(
        &mut self,
        event: XbxEngineHostVideoFrameDropEvent,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().record_host_video_frame_drop(event)
    }

    fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<XbxEngineRenderFrame>, XbxEngineRuntimeError> {
        self.as_mut().take_latest_render_frame()
    }

    fn record_video_frame_drop(
        &mut self,
        observation: XbxEngineVideoFrameDropObservation,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().record_video_frame_drop(observation)
    }

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().request_video_keyframe()
    }

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().request_decoder_reset()
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.as_mut().stop()
    }
}

pub struct PlaceholderXbxEngineMediaBackend {
    input_backend: Box<dyn XbxEngineInputBackend>,
    pub negotiation_count: usize,
    pub last_offer_sdp: Option<String>,
    pub last_answer_sdp: Option<String>,
    pub last_remote_candidates: Vec<XbxEngineIceCandidateDto>,
    pub last_display_state: Option<XbxEngineDisplayStateDto>,
    pub audio_volume: f32,
    pub microphone_capturing: bool,
    pub keyboard_pointer_enabled: bool,
    pub last_keyboard_pointer_event: Option<XbxEngineInputEventDto>,
    pub last_pressed_controller_button: Option<(String, u64)>,
    pub last_input_status: XbxEngineInputStatus,
    pub last_runtime_stats: XbxEngineMediaRuntimeStats,
    pub pending_runtime_recovery_action: Option<XbxEnginePendingRuntimeRecoveryAction>,
}

impl Default for PlaceholderXbxEngineMediaBackend {
    fn default() -> Self {
        Self::with_input_backend(Box::<NoopXbxEngineInputBackend>::default())
    }
}

impl PlaceholderXbxEngineMediaBackend {
    pub fn with_input_backend(input_backend: Box<dyn XbxEngineInputBackend>) -> Self {
        Self {
            input_backend,
            negotiation_count: 0,
            last_offer_sdp: None,
            last_answer_sdp: None,
            last_remote_candidates: Vec::new(),
            last_display_state: None,
            audio_volume: 1.0,
            microphone_capturing: false,
            keyboard_pointer_enabled: false,
            last_keyboard_pointer_event: None,
            last_pressed_controller_button: None,
            last_input_status: XbxEngineInputStatus::default(),
            last_runtime_stats: XbxEngineMediaRuntimeStats::default(),
            pending_runtime_recovery_action: None,
        }
    }
}

impl XbxEngineMediaBackend for PlaceholderXbxEngineMediaBackend {
    fn sync_runtime_config(
        &mut self,
        _runtime_config: &XbxEngineRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn negotiate(
        &mut self,
        request: XbxEngineMediaNegotiationRequest,
    ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError> {
        self.negotiation_count += 1;
        self.last_input_status = self
            .input_backend
            .attach_session(&request.session.session_id)?;

        let offer_sdp = if request.restart {
            format!(
                "v=0\r\no={} restart-placeholder:{}\r\n",
                request.session.session_id, self.negotiation_count
            )
        } else {
            format!(
                "v=0\r\no={} initial-placeholder:{}\r\n",
                request.session.session_id, self.negotiation_count
            )
        };
        self.last_offer_sdp = Some(offer_sdp.clone());

        let frame_clock = now_ms_f64();
        self.last_runtime_stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_frame: Some(XbxEngineVideoFrameStats {
                width: 1280,
                height: 720,
                frame_seq: self.negotiation_count as u64,
                fps: 60.0,
                rendered_at_ms: frame_clock + 12.0,
            }),
            ..Default::default()
        };
        // 让默认测试后端也具备一条最小可用的 ICE 候选，避免 runtime 依赖空交换兜底。
        let local_candidate = XbxEngineIceCandidateDto {
            candidate: "candidate:placeholder 1 udp 2130706431 127.0.0.1 60000 typ host"
                .to_string(),
            sdp_m_line_index: Some(0),
            sdp_mid: Some("0".to_string()),
        };
        Ok(XbxEngineMediaNegotiation {
            local_offer_sdp: offer_sdp,
            local_candidates: vec![local_candidate],
            surface_id: format!("surface:{}", request.viewport.viewport_id),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: Some(frame_clock),
            frame_decoded_time_ms: Some(frame_clock + 8.0),
            frame_rendered_time_ms: Some(frame_clock + 12.0),
            input_status: self.last_input_status.clone(),
        })
    }

    fn apply_remote_description(
        &mut self,
        answer_sdp: String,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_answer_sdp = Some(answer_sdp);
        self.last_remote_candidates = remote_candidates;
        Ok(())
    }

    fn add_remote_ice_candidates(
        &mut self,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_remote_candidates.extend(remote_candidates);
        Ok(())
    }

    fn local_candidates_snapshot(
        &self,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        Ok(Vec::new())
    }

    fn local_ice_gathering_complete(&self) -> Result<bool, XbxEngineRuntimeError> {
        Ok(true)
    }

    fn create_offer(&mut self) -> Result<String, XbxEngineRuntimeError> {
        let next_offer = format!(
            "v=0\r\no=placeholder chat-offer:{}\r\n",
            self.negotiation_count.saturating_add(1)
        );
        self.last_offer_sdp = Some(next_offer.clone());
        Ok(next_offer)
    }

    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_display_state = Some(state);
        Ok(())
    }

    fn set_audio_volume(&mut self, value: f32) -> Result<(), XbxEngineRuntimeError> {
        self.audio_volume = value;
        Ok(())
    }

    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError> {
        self.microphone_capturing = capturing;
        Ok(())
    }

    fn press_controller_button(
        &mut self,
        button: String,
        duration_ms: u64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_pressed_controller_button = Some((button, duration_ms));
        Ok(())
    }

    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError> {
        self.keyboard_pointer_enabled = enabled;
        Ok(())
    }

    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_keyboard_pointer_event = Some(event);
        Ok(())
    }

    fn current_input_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(self.last_input_status.clone())
    }

    fn snapshot_runtime_stats(&self) -> Result<XbxEngineMediaRuntimeStats, XbxEngineRuntimeError> {
        Ok(self.last_runtime_stats.clone())
    }

    fn update_host_video_present_metrics(
        &mut self,
        metrics: XbxEngineHostVideoPresentMetrics,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_runtime_stats.latest_host_mailbox_submit_time_ms =
            metrics.latest_host_submit_time_ms;
        self.last_runtime_stats.latest_video_host_present_time_ms =
            metrics.latest_host_present_time_ms;
        self.last_runtime_stats.host_mailbox_submit_epoch = metrics.host_mailbox_submit_epoch;
        self.last_runtime_stats.host_display_tick_epoch = metrics.host_display_tick_epoch;
        self.last_runtime_stats.host_frame_present_epoch = metrics.host_frame_present_epoch;
        self.last_runtime_stats.host_cadence_phase = metrics.cadence_phase;
        self.last_runtime_stats.video_present_fps = metrics.present_fps;
        self.last_runtime_stats.host_mailbox_enqueue_count_total =
            metrics.host_mailbox_enqueue_count_total;
        self.last_runtime_stats.host_mailbox_drop_count_total =
            metrics.host_mailbox_drop_count_total;
        self.last_runtime_stats.host_mailbox_overwrite_count_total =
            metrics.host_mailbox_overwrite_count_total;
        self.last_runtime_stats.video_present_descriptor_upload_mode =
            metrics.descriptor_upload_mode;
        self.last_runtime_stats
            .video_present_descriptor_metal_import_count_total =
            metrics.descriptor_metal_import_count_total;
        self.last_runtime_stats
            .video_present_descriptor_cpu_upload_count_total =
            metrics.descriptor_cpu_upload_count_total;
        Ok(())
    }

    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Result<Option<XbxEnginePendingRuntimeRecoveryAction>, XbxEngineRuntimeError> {
        Ok(self.pending_runtime_recovery_action.take())
    }

    fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<XbxEngineRenderFrame>, XbxEngineRuntimeError> {
        Ok(None)
    }

    fn record_video_frame_drop(
        &mut self,
        observation: XbxEngineVideoFrameDropObservation,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.last_runtime_stats.latest_video_frame_drop = Some(observation);
        Ok(())
    }

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
}

fn now_ms_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "backend.test.rs"]
mod backend_tests;
