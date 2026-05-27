use crate::media::video::h264::inspection::H264AccessUnitInspection;
use crate::{
    XbxEngineH264InspectionObservation,
    XbxEngineKeyframeRequestEpisodeObservation as XbxEnginePictureRecoveryEpisodeObservation,
    XbxEngineMediaRuntimeStats, XbxEngineVideoTimelineGapSnapshot,
    XbxEngineVideoTimelineObservation,
};

/// 恢复系统的统一“事实模型”合同（L0 跨层语义源）。
///
/// 层边界（禁止并行裁决同一故障）：
/// - L1 Insert：`displayed_idr_serving_from_stats` → pre-decode Emit/Hold 窄路径
/// - L2 Decode：本地 FSM + `insert_emit_bootstrap_bypass`；不重复 bootstrap 主闸
/// - L3 Owner：`displayed_idr_serving_allows_relaxed_controls_from_stats` → 控制面短脉冲抑制
/// - L0 本模块：`recovery_supply_break_active_from_stats`、`derive_recovery_surface_phase_from_stats`
///   → Owner/policy/coordinator 只读；禁止在 owner 内再写平行 supply-break 阈值
/// - L4 Action：receive PLI / session reset；session RequestPli 仅 defer
///
/// 注意：枚举值跨层共享，不允许在其他模块平行定义同名语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameValue {
    #[allow(dead_code)]
    Disposable,
    Continuity,
    Reference,
    RecoveryAnchor,
    #[allow(dead_code)]
    CleanAnchor,
}

impl FrameValue {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Disposable => "Disposable",
            Self::Continuity => "Continuity",
            Self::Reference => "Reference",
            Self::RecoveryAnchor => "RecoveryAnchor",
            Self::CleanAnchor => "CleanAnchor",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GapSeverity {
    /// 低价值 / 无活跃可修补缺口的基线。
    LowValueGap,
    /// 有缺口但尚无 reference 级证据，不得直接进入恢复主线当 ReferenceGap。
    RepairableGap,
    ReferenceGap,
    AnchorGap,
    ChainBroken,
    RecoveryBlocked,
}

impl GapSeverity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LowValueGap => "LowValueGap",
            Self::RepairableGap => "RepairableGap",
            Self::ReferenceGap => "ReferenceGap",
            Self::AnchorGap => "AnchorGap",
            Self::ChainBroken => "ChainBroken",
            Self::RecoveryBlocked => "RecoveryBlocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryEpisodeStage {
    Requested,
    Sent,
    ResponseObserved,
    Decoded,
    Deferred,
    Expired,
}

/// RFC: 恢复进度统一七级语义，作为跨层事实口径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryProgressLevel {
    WaitingResponse,
    ContinuationSeen,
    AnchorSeen,
    Decoded,
    PlaybackRecovered,
    CleanAnchorCommitted,
    DisplayStable,
}

impl RecoveryProgressLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WaitingResponse => "WaitingResponse",
            Self::ContinuationSeen => "ContinuationSeen",
            Self::AnchorSeen => "AnchorSeen",
            Self::Decoded => "Decoded",
            Self::PlaybackRecovered => "PlaybackRecovered",
            Self::CleanAnchorCommitted => "CleanAnchorCommitted",
            Self::DisplayStable => "DisplayStable",
        }
    }
}

pub(crate) fn recovery_progress_level_from_str(value: &str) -> Option<RecoveryProgressLevel> {
    match value {
        "WaitingResponse" => Some(RecoveryProgressLevel::WaitingResponse),
        "ContinuationSeen" => Some(RecoveryProgressLevel::ContinuationSeen),
        "AnchorSeen" => Some(RecoveryProgressLevel::AnchorSeen),
        "Decoded" => Some(RecoveryProgressLevel::Decoded),
        "PlaybackRecovered" => Some(RecoveryProgressLevel::PlaybackRecovered),
        "CleanAnchorCommitted" => Some(RecoveryProgressLevel::CleanAnchorCommitted),
        "DisplayStable" => Some(RecoveryProgressLevel::DisplayStable),
        _ => None,
    }
}

impl RecoveryEpisodeStage {
    #[allow(dead_code)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "Requested",
            Self::Sent => "Sent",
            Self::ResponseObserved => "ResponseObserved",
            Self::Decoded => "Decoded",
            Self::Deferred => "Deferred",
            Self::Expired => "Expired",
        }
    }
}

pub(crate) fn recovery_episode_stage_from_status(status: &str) -> Option<RecoveryEpisodeStage> {
    match status {
        "requested" => Some(RecoveryEpisodeStage::Requested),
        "sent" => Some(RecoveryEpisodeStage::Sent),
        "response-observed" | "packet-seen" => Some(RecoveryEpisodeStage::ResponseObserved),
        "decoded" => Some(RecoveryEpisodeStage::Decoded),
        "deferred" => Some(RecoveryEpisodeStage::Deferred),
        "expired-unsent" | "missed" => Some(RecoveryEpisodeStage::Expired),
        _ => None,
    }
}

pub(crate) fn recovery_progress_level_from_episode(
    status: &str,
    response_verdict: Option<&str>,
    first_video_packet_is_keyframe: Option<bool>,
    first_keyframe_packet_at_ms: Option<f64>,
    first_keyframe_decoded_at_ms: Option<f64>,
    has_current_clean_anchor: bool,
    has_display_stable: bool,
) -> Option<RecoveryProgressLevel> {
    if has_display_stable {
        return Some(RecoveryProgressLevel::DisplayStable);
    }
    if has_current_clean_anchor || response_verdict == Some("cleanAnchorCommitted") {
        return Some(RecoveryProgressLevel::CleanAnchorCommitted);
    }
    if first_keyframe_decoded_at_ms.is_some() || status == "decoded" {
        return Some(RecoveryProgressLevel::Decoded);
    }
    if first_keyframe_packet_at_ms.is_some()
        || first_video_packet_is_keyframe == Some(true)
        || matches!(status, "packet-seen")
    {
        return Some(RecoveryProgressLevel::AnchorSeen);
    }
    if matches!(status, "response-observed")
        || (first_video_packet_is_keyframe == Some(false) && response_verdict != Some("pending"))
    {
        return Some(RecoveryProgressLevel::ContinuationSeen);
    }
    if matches!(
        status,
        "requested" | "sent" | "deferred" | "failed" | "expired-unsent" | "missed"
    ) {
        return Some(RecoveryProgressLevel::WaitingResponse);
    }
    None
}

pub(crate) fn recovery_progress_missing_anchor(progress: Option<RecoveryProgressLevel>) -> bool {
    matches!(
        progress,
        Some(RecoveryProgressLevel::WaitingResponse | RecoveryProgressLevel::ContinuationSeen)
            | None
    )
}

pub(crate) fn recovery_progress_allows_decoder_reset(
    progress: Option<RecoveryProgressLevel>,
) -> bool {
    matches!(
        progress,
        Some(
            RecoveryProgressLevel::AnchorSeen
                | RecoveryProgressLevel::Decoded
                | RecoveryProgressLevel::PlaybackRecovered
                | RecoveryProgressLevel::CleanAnchorCommitted
                | RecoveryProgressLevel::DisplayStable
        )
    )
}

const FRESH_H264_IDR_ADMISSION_MS: f64 = 3_000.0;
/// 与 owner TimedFallback 对齐：尽早结束 transport-await 焊死并触发续播窄路径。
const RECOVERY_EXIT_TIMED_FALLBACK_SUBMIT_AGE_MS: f64 = 1_500.0;
const GAP_KEYFRAME_ONLY_MAX_AGE_MS: f64 = 1_500.0;
const GAP_ABANDON_KEYFRAME_ONLY_MS: f64 = 5_000.0;
/// PS/config 变更后短窗：非 IDR 不进解码器，优先要 IDR。
const PARAMETER_SETS_CHANGE_STRICT_MIN_MS: f64 = 400.0;
const PARAMETER_SETS_CHANGE_STRICT_MAX_MS: f64 = 2_000.0;

/// 近期 inspection 已接纳 IDR（trace `h264IdrAccessUnitObserved` 同源条件）。
pub(crate) fn fresh_h264_idr_admission_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            inspection.is_idr
                && inspection.admission_accepted
                && (now_ms - inspection.observed_at_ms).max(0.0) <= FRESH_H264_IDR_ADMISSION_MS
        })
}

/// TimedFallback 下 submit/decode 双停滞：需本地 decoder reset 打通续播窄路径（跳过 burst 等待）。
pub(crate) const TIMED_FALLBACK_DECODER_UNSTICK_SUBMIT_AGE_MS: f64 = 4_000.0;
pub(crate) const TIMED_FALLBACK_DECODER_UNSTICK_DECODE_AGE_MS: f64 = 4_000.0;

pub(crate) fn timed_fallback_decoder_unstick_required_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if !recovery_timed_fallback_active_from_stats(stats, now_ms) {
        return false;
    }
    if !displayed_idr_serving_from_stats(stats) {
        return false;
    }
    if stats.video_decoder_recovery_state.as_deref() != Some("waiting-keyframe") {
        return false;
    }
    let submit_stale = stats
        .submit_age_ms
        .is_some_and(|age| age >= TIMED_FALLBACK_DECODER_UNSTICK_SUBMIT_AGE_MS);
    let decode_stale = stats.video_decoder_stalled.unwrap_or(false)
        || stats.latest_video_decode_ok_time_ms.is_none_or(|at_ms| {
            (now_ms - at_ms).max(0.0) >= TIMED_FALLBACK_DECODER_UNSTICK_DECODE_AGE_MS
        });
    submit_stale && decode_stale
}

/// waiting-keyframe 且无 IDR 进展时禁止本地 decoder reset（Reconfigure 等显式路径除外）。
pub(crate) fn decoder_reset_permitted_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    progress: Option<RecoveryProgressLevel>,
    now_ms: f64,
    allow_waiting_keyframe_bypass: bool,
) -> bool {
    if allow_waiting_keyframe_bypass
        || timed_fallback_decoder_unstick_required_from_stats(stats, now_ms)
        || recovery_supply_break_active_from_stats(stats, now_ms)
    {
        return true;
    }
    if has_current_clean_anchor_from_stats(stats)
        && displayed_idr_serving_from_stats(stats)
        && stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe")
        && stats
            .submit_age_ms
            .is_some_and(|age| age >= TIMED_FALLBACK_DECODER_UNSTICK_SUBMIT_AGE_MS)
    {
        return true;
    }
    if stats.video_decoder_recovery_state.as_deref() != Some("waiting-keyframe") {
        return true;
    }
    if fresh_h264_idr_admission_from_stats(stats, now_ms) {
        return true;
    }
    recovery_progress_allows_decoder_reset(progress)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RecoveryExitPath {
    HostIdr,
    DecodeOutput,
    TimedFallback,
    #[default]
    AwaitingAnchor,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecoveryExitThresholds {
    pub(crate) degraded_decode_age_ms: f64,
    pub(crate) timed_fallback_submit_age_ms: f64,
}

impl Default for RecoveryExitThresholds {
    fn default() -> Self {
        Self {
            degraded_decode_age_ms: 1_200.0,
            timed_fallback_submit_age_ms: RECOVERY_EXIT_TIMED_FALLBACK_SUBMIT_AGE_MS,
        }
    }
}

/// 恢复退出用的 host IDR 证据：不接受「历史上屏过」的 stale displayed-idr 单独挡 TimedFallback。
fn recovery_exit_host_idr_path_active(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    if fresh_h264_idr_admission_from_stats(stats, now_ms) {
        return true;
    }
    let display = RecoveryDisplayFacts::from_stats(stats);
    if display.fresh_anchor_recovered_at_ms.is_some() {
        return true;
    }
    stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch)
        && stats.video_anchor_clean_observed_at_ms.is_some()
}

/// 恢复会话退出 `receiverWaitingKeyframe` 焊死：新鲜 IDR/锚点 → decode 输出 → 超时降级。
pub(crate) fn recovery_exit_path_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    thresholds: RecoveryExitThresholds,
) -> RecoveryExitPath {
    let waiting_keyframe =
        stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe");
    let submit_stalled = stats
        .submit_age_ms
        .is_some_and(|age| age >= thresholds.timed_fallback_submit_age_ms);
    if waiting_keyframe && submit_stalled && twcc_healthy_for_recovery_fallback(stats) {
        return RecoveryExitPath::TimedFallback;
    }
    if recovery_exit_host_idr_path_active(stats, now_ms) {
        return RecoveryExitPath::HostIdr;
    }
    let decode_fresh = stats
        .latest_video_decode_ok_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= thresholds.degraded_decode_age_ms);
    let host_output_advancing =
        stats.host_frame_present_epoch > 0 && stats.recovery_playback_recovered_at_ms.is_some();
    let submit_pipeline_active = stats
        .submit_age_ms
        .map(|age| age < thresholds.timed_fallback_submit_age_ms)
        .unwrap_or(true);
    if decode_fresh && host_output_advancing && submit_pipeline_active {
        return RecoveryExitPath::DecodeOutput;
    }
    RecoveryExitPath::AwaitingAnchor
}

pub(crate) fn recovery_exit_trace_await_suffix(path: RecoveryExitPath) -> &'static str {
    match path {
        RecoveryExitPath::HostIdr => "hostIdrOrCleanAnchor",
        RecoveryExitPath::DecodeOutput => "decodeOutput",
        RecoveryExitPath::TimedFallback => "timedFallback",
        RecoveryExitPath::AwaitingAnchor => "hostIdrOrCleanAnchor",
    }
}

fn twcc_healthy_for_recovery_fallback(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.latest_video_twcc_observation.as_ref().map_or(
        stats.transport_state == xbxengine_protocol::XbxEngineTransportStateDto::Connected,
        |twcc| {
            twcc.twcc_sample_valid && twcc.packet_loss_ratio <= 0.08 && twcc.delivery_ratio >= 0.92
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GapVsKeyframeMode {
    RepairFirst,
    KeyframeOnly,
    AbandonGap,
}

pub(crate) fn parameter_sets_change_strict_window_ms(effective_rtt_ms: f64) -> f64 {
    let rtt_ms = effective_rtt_ms.clamp(20.0, 400.0);
    (4.0 * rtt_ms)
        .max(PARAMETER_SETS_CHANGE_STRICT_MIN_MS)
        .min(PARAMETER_SETS_CHANGE_STRICT_MAX_MS)
}

/// PS/config 变更后的短窗：修洞期应优先 IDR，不再宽进 soft missing-IDR delta。
pub(crate) fn parameter_sets_change_strict_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    effective_rtt_ms: f64,
) -> bool {
    if fresh_h264_idr_admission_from_stats(stats, now_ms) {
        return false;
    }
    if !displayed_idr_serving_from_stats(stats) {
        return false;
    }
    if media_supply_priming_from_stats(stats, now_ms) {
        return false;
    }
    let window_ms = parameter_sets_change_strict_window_ms(effective_rtt_ms);
    stats
        .video_parameter_sets_changed_at_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) < window_ms)
}

fn repairing_missing_idr_keyframe_pressure_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if fresh_h264_idr_admission_from_stats(stats, now_ms) {
        return false;
    }
    if !displayed_idr_serving_from_stats(stats) {
        return false;
    }
    let receiver_repairing = stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| obs.receiver_state == "repairing");
    if !receiver_repairing {
        return false;
    }
    let missing_idr = stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            !inspection.is_idr
                && is_soft_missing_idr_bootstrap_reject_reason(
                    inspection.bootstrap_reject_reason.as_deref(),
                )
        });
    let gap_open = stats
        .latest_video_timeline_observation
        .as_ref()
        .and_then(|timeline| timeline.gap.as_ref())
        .is_some();
    missing_idr
        && (gap_open
            || stats
                .latest_video_receiver_observation
                .as_ref()
                .is_some_and(|obs| obs.keyframe_request_pending || obs.nack_in_flight))
}

pub(crate) fn resolve_gap_vs_keyframe_mode(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    effective_rtt_ms: f64,
) -> GapVsKeyframeMode {
    let decoder_waiting = stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe");
    let gap_age_ms = stats
        .latest_video_timeline_observation
        .as_ref()
        .and_then(|timeline| timeline.gap.as_ref())
        .map(|gap| (now_ms - gap.observed_at_ms).max(0.0));
    let gap_stale = gap_age_ms
        .is_some_and(|age| age >= GAP_KEYFRAME_ONLY_MAX_AGE_MS.max(effective_rtt_ms * 2.0));
    let idr_pressure =
        parameter_sets_change_strict_active_from_stats(stats, now_ms, effective_rtt_ms)
            || repairing_missing_idr_keyframe_pressure_from_stats(stats, now_ms);
    if decoder_waiting || gap_stale || idr_pressure {
        if gap_age_ms.is_some_and(|age| age >= GAP_ABANDON_KEYFRAME_ONLY_MS) {
            return GapVsKeyframeMode::AbandonGap;
        }
        return GapVsKeyframeMode::KeyframeOnly;
    }
    GapVsKeyframeMode::RepairFirst
}

pub(crate) fn gap_keyframe_only_mode_active(mode: GapVsKeyframeMode) -> bool {
    matches!(
        mode,
        GapVsKeyframeMode::KeyframeOnly | GapVsKeyframeMode::AbandonGap
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoalescingMode {
    Merge,
    Refresh,
}

impl CoalescingMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Refresh => "Refresh",
        }
    }
}

const TRANSPORT_AWAIT_UNRESOLVED_REASONS: [&str; 4] = [
    "receiverWaitingKeyframe",
    "awaitingRecoveryAnchor",
    "awaitRecoveryAnchor",
    "referenceChainUnrecoverable",
];

pub(crate) fn is_transport_await_unresolved_reason(reason: &str) -> bool {
    TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason)
}

pub(crate) fn is_transport_await_probe_source_event(source_event: Option<&str>) -> bool {
    matches!(
        source_event,
        Some(
            "frame-await-recovery-anchor"
                | "frame-inspection-rejected-await-anchor"
                | "frame-inspection-rejected-trigger-recovery-anchor"
        )
    )
}

pub(crate) fn is_invalid_recovery_bootstrap_reject_reason(reason: Option<&str>) -> bool {
    matches!(
        reason,
        Some("bootstrapMissingSps" | "bootstrapMissingPps" | "inspectionRejectInvalidSliceHeader")
    )
}

pub(crate) fn is_recovery_delta_continuation_ready(inspection: &H264AccessUnitInspection) -> bool {
    inspection.slice_headers_valid
        && inspection.delta_continuation_ready()
        && inspection.committed_sps_present()
        && inspection.committed_pps_present()
}

pub(crate) fn inspection_has_invalid_recovery_bootstrap(
    inspection: &XbxEngineH264InspectionObservation,
) -> bool {
    !inspection.bootstrap_ready
        && is_invalid_recovery_bootstrap_reject_reason(
            inspection.bootstrap_reject_reason.as_deref(),
        )
}

const CURRENT_TRANSPORT_AWAIT_INVALID_BOOTSTRAP_FRESH_MS: f64 = 220.0;

pub(crate) fn is_terminal_transport_await_deferred_episode(
    episode: &XbxEnginePictureRecoveryEpisodeObservation,
    inspection: Option<&XbxEngineH264InspectionObservation>,
    has_clean_anchor_evidence: bool,
    now_ms: f64,
    fresh_window_ms: f64,
) -> bool {
    if !matches!(
        episode.request_reason.as_deref(),
        Some("receiverWaitingKeyframe")
    ) {
        return false;
    }
    let stage = recovery_episode_stage_from_status(episode.status.as_str());
    if !matches!(
        stage,
        Some(RecoveryEpisodeStage::Deferred | RecoveryEpisodeStage::Expired)
    ) {
        return false;
    }
    if episode.response_verdict.as_deref() != Some("transportDeferred") {
        return false;
    }
    if episode.sent_at_ms.is_some() || !has_clean_anchor_evidence {
        return false;
    }
    let Some(inspection) = inspection else {
        return false;
    };
    if (now_ms - inspection.observed_at_ms).max(0.0) > fresh_window_ms {
        return false;
    }
    if !inspection_has_invalid_recovery_bootstrap(inspection) {
        return false;
    }
    match (
        episode.response_rtp_timestamp,
        inspection.frame_rtp_timestamp,
    ) {
        (Some(response_ts), Some(frame_ts)) => frame_ts == response_ts,
        _ => inspection.observed_at_ms >= episode.requested_at_ms,
    }
}

pub(crate) fn is_receiver_state_receiving(receiver_state: Option<&str>) -> bool {
    matches!(receiver_state, Some("receiving"))
}

/// 与 RFC 四态一致：优先 `receiver_observation`，其次 timeline `chain.state`。
pub(crate) fn is_timeline_chain_receiving_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    if stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| is_receiver_state_receiving(Some(obs.receiver_state.as_str())))
    {
        return true;
    }
    stats
        .latest_video_timeline_observation
        .as_ref()
        .is_some_and(|timeline| matches!(timeline.chain.state.as_str(), "receiving"))
}

pub(crate) fn is_receiver_state_waiting_keyframe(receiver_state: Option<&str>) -> bool {
    matches!(receiver_state, Some("waiting-keyframe"))
}

pub(crate) fn is_receiver_state_repairing(receiver_state: Option<&str>) -> bool {
    matches!(receiver_state, Some("repairing"))
}

pub(crate) fn is_ingress_waiting_keyframe(
    receiver_state: Option<&str>,
    chain_state: Option<&str>,
    chain_reason: Option<&str>,
    source_event: Option<&str>,
) -> bool {
    if is_receiver_state_receiving(receiver_state) || is_receiver_state_repairing(receiver_state) {
        return false;
    }
    if is_receiver_state_waiting_keyframe(receiver_state) {
        return true;
    }
    if matches!(chain_state, Some("receiving" | "priming")) {
        return false;
    }
    let probe_event_waiting = is_transport_await_probe_source_event(source_event)
        && !matches!(chain_state, Some("receiving" | "priming"));
    matches!(chain_state, Some("waiting-keyframe"))
        || chain_reason.is_some_and(is_transport_await_unresolved_reason)
        || probe_event_waiting
}

pub(crate) fn has_unresolved_transport_await_issue_from_observation(
    timeline: &XbxEngineVideoTimelineObservation,
) -> bool {
    if matches!(timeline.chain.state.as_str(), "receiving" | "priming") {
        return false;
    }
    if timeline
        .chain
        .reason
        .as_deref()
        .is_some_and(is_transport_await_unresolved_reason)
    {
        return true;
    }
    if timeline
        .frame
        .as_ref()
        .and_then(|frame| frame.close_reason.as_deref())
        .is_some_and(is_transport_await_unresolved_reason)
    {
        return true;
    }
    timeline.gap.as_ref().is_some_and(|gap| {
        !matches!(gap.state.as_str(), "resolved" | "expired")
            && timeline
                .chain
                .reason
                .as_deref()
                .is_some_and(is_transport_await_unresolved_reason)
    })
}

pub(crate) fn current_clean_anchor_observed_at_ms(
    clean_anchor_epoch: Option<u64>,
    clean_anchor_observed_at_ms: Option<f64>,
    clean_anchor_source_event: Option<&str>,
    recovery_epoch: u64,
) -> Option<f64> {
    if clean_anchor_epoch == Some(recovery_epoch)
        && clean_anchor_source_event == Some("displayed-idr")
    {
        clean_anchor_observed_at_ms
    } else {
        None
    }
}

pub(crate) fn current_clean_anchor_bridge_observed_at_ms(
    bridge_epoch: Option<u64>,
    bridge_observed_at_ms: Option<f64>,
    bridge_source_event: Option<&str>,
    recovery_epoch: u64,
) -> Option<f64> {
    if bridge_epoch == Some(recovery_epoch)
        && bridge_source_event == Some("hostVisibleAnchorPending")
    {
        bridge_observed_at_ms
    } else {
        None
    }
}

/// Post-decode 恢复显示事实（host present 单点写入）；控制面读路径统一经此投影。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RecoveryDisplayFacts {
    pub displayed_idr_rtp: Option<u32>,
    pub displayed_idr_at_ms: Option<f64>,
    pub playback_recovered_at_ms: Option<f64>,
    pub fresh_anchor_recovered_at_ms: Option<f64>,
}

impl RecoveryDisplayFacts {
    pub(crate) fn from_stats(stats: &XbxEngineMediaRuntimeStats) -> Self {
        Self {
            displayed_idr_rtp: stats.recovery_displayed_idr_rtp,
            displayed_idr_at_ms: stats.recovery_displayed_idr_at_ms,
            playback_recovered_at_ms: stats.recovery_playback_recovered_at_ms,
            fresh_anchor_recovered_at_ms: stats.recovery_fresh_anchor_recovered_at_ms,
        }
    }

    pub(crate) fn has_established_displayed_idr(self) -> bool {
        self.displayed_idr_at_ms.is_some()
    }
}

pub(crate) fn current_clean_anchor_observed_at_ms_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<f64> {
    current_clean_anchor_observed_at_ms(
        stats.video_anchor_clean_epoch,
        stats.video_anchor_clean_observed_at_ms,
        stats.video_anchor_clean_source_event.as_deref(),
        stats.transport_recovery_epoch,
    )
}

pub(crate) fn has_current_clean_anchor_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    let display = RecoveryDisplayFacts::from_stats(stats);
    display.fresh_anchor_recovered_at_ms.is_some() || display.has_established_displayed_idr()
}

/// latest-only mailbox 上屏帧常已是 IDR 之后的 delta；pending IDR + host 已 present 即视为 serving。
pub(crate) fn displayed_idr_serving_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.recovery_displayed_idr_at_ms.is_some()
        || (stats.recovery_pending_displayed_idr_rtp.is_some()
            && stats.host_frame_present_epoch > 0)
}

/// host present 提交 displayed-idr 事实时优先用 decode 侧 pending IDR，而非当前 displayed delta RTP。
pub(crate) fn resolve_host_display_idr_anchor_rtp(
    stats: &XbxEngineMediaRuntimeStats,
    last_displayed_rtp: Option<u32>,
) -> Option<u32> {
    stats
        .recovery_pending_displayed_idr_rtp
        .or(last_displayed_rtp)
}

const DISPLAYED_IDR_SERVING_DECODER_BOOTSTRAP_FRESH_MS: f64 = 1_500.0;
pub(crate) const DISPLAYED_IDR_SERVING_STALE_SUBMIT_BREAK_MS: f64 = 1_000.0;

/// steady continuation 的 codec 元数据拒因；displayed-idr 已 serving 时不应切断窄路径放松。
pub(crate) fn is_soft_missing_idr_bootstrap_reject_reason(reason: Option<&str>) -> bool {
    matches!(reason, Some("bootstrapMissingIdr" | "NonIdrVcl"))
}

/// TimedFallback：submit 已停滞但 TWCC 健康，允许 displayed-idr 续播窄路径（不等新 IDR AU）。
pub(crate) fn recovery_timed_fallback_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    recovery_exit_path_from_stats(stats, now_ms, RecoveryExitThresholds::default())
        == RecoveryExitPath::TimedFallback
}

/// decoder 要 IDR / bootstrap 硬拒 / submit 管线停滞时，禁用 P1 放松（collapse、强制 Submit、抑制 recovery-wait）。
pub(crate) fn displayed_idr_serving_relaxation_blocked_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if transport_await_has_hard_bootstrap_evidence_from_stats(stats, now_ms) {
        return true;
    }
    if decoder_bootstrap_blocks_displayed_idr_relaxation(stats, now_ms) {
        return true;
    }
    // 已有 clean anchor 时不得因 decode FSM 焊死 waiting-keyframe 而切断 Insert 续播窄路径。
    if has_current_clean_anchor_from_stats(stats) && displayed_idr_serving_from_stats(stats) {
        return false;
    }
    if stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe") {
        if recovery_timed_fallback_active_from_stats(stats, now_ms)
            && displayed_idr_serving_from_stats(stats)
        {
            return false;
        }
        if recovery_supply_break_active_from_stats(stats, now_ms) {
            return false;
        }
        return true;
    }
    stale_submit_pipeline_breaks_displayed_idr_relaxation(stats, now_ms)
}

/// displayed IDR 已上屏且允许 P1 放松控制（短脉冲抑制，不含供给断裂长尾）。
pub(crate) fn displayed_idr_serving_allows_relaxed_controls_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    displayed_idr_serving_from_stats(stats)
        && !displayed_idr_serving_relaxation_blocked_from_stats(stats, now_ms)
}

fn decoder_bootstrap_blocks_displayed_idr_relaxation(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    let Some(observation) = stats
        .latest_video_decoder_bootstrap_gate_observation
        .as_ref()
        .filter(|observation| {
            (now_ms - observation.observed_at_ms).max(0.0)
                <= DISPLAYED_IDR_SERVING_DECODER_BOOTSTRAP_FRESH_MS
        })
    else {
        return false;
    };
    if !observation.bootstrap_ready
        && is_soft_missing_idr_bootstrap_reject_reason(
            observation.bootstrap_reject_reason.as_deref(),
        )
        && displayed_idr_serving_from_stats(stats)
    {
        return false;
    }
    !observation.bootstrap_ready
        && is_soft_missing_idr_bootstrap_reject_reason(
            observation.bootstrap_reject_reason.as_deref(),
        )
}

fn stale_submit_pipeline_breaks_displayed_idr_relaxation(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if recovery_timed_fallback_active_from_stats(stats, now_ms) {
        return false;
    }
    if recovery_supply_break_active_from_stats(stats, now_ms) {
        return false;
    }
    stats
        .submit_age_ms
        .is_some_and(|age_ms| age_ms >= DISPLAYED_IDR_SERVING_STALE_SUBMIT_BREAK_MS)
        && (stats.video_renderer_stalled.unwrap_or(false)
            || stats.video_decoder_stalled.unwrap_or(false))
}

/// displayed IDR 已上屏且仍在 gap repair：不把 receiver 投影成 waiting-keyframe，避免 supply 短脉冲。
pub(crate) fn should_collapse_receiver_waiting_keyframe_to_repairing(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    has_active_gap: bool,
    assembled_frame_count: u64,
) -> bool {
    displayed_idr_serving_allows_relaxed_controls_from_stats(stats, now_ms)
        && has_active_gap
        && assembled_frame_count > 0
}

const TRANSPORT_AWAIT_HARD_BOOTSTRAP_FRESH_MS: f64 = 1_500.0;

/// transport-await 硬证据：仅 receiver/inspection/display 事实，不读 keyframe episode terminal。
pub(crate) fn transport_await_has_hard_bootstrap_evidence_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    let Some(inspection) = stats
        .latest_h264_inspection_observation
        .as_ref()
        .filter(|inspection| {
            (now_ms - inspection.observed_at_ms).max(0.0) <= TRANSPORT_AWAIT_HARD_BOOTSTRAP_FRESH_MS
        })
    else {
        return false;
    };
    if inspection_has_invalid_recovery_bootstrap(inspection) {
        return true;
    }
    if !inspection.bootstrap_ready {
        return match inspection.bootstrap_reject_reason.as_deref() {
            Some("bootstrapMissingSps" | "bootstrapMissingPps" | "bootstrapInvalidSliceHeader") => {
                true
            }
            Some("bootstrapMissingIdr" | "NonIdrVcl") => !displayed_idr_serving_from_stats(stats),
            _ => false,
        };
    }
    false
}

pub(crate) fn has_current_transport_await_issue_from_observation(
    timeline: &XbxEngineVideoTimelineObservation,
    current_clean_anchor_observed_at_ms: Option<f64>,
) -> bool {
    has_unresolved_transport_await_issue_from_observation(timeline)
        && current_clean_anchor_observed_at_ms
            .is_none_or(|clean_anchor_at_ms| timeline.observed_at_ms > clean_anchor_at_ms)
}

pub(crate) fn has_current_transport_await_issue_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
) -> bool {
    let current_clean_anchor_observed_at_ms = current_clean_anchor_observed_at_ms_from_stats(stats);
    let Some(timeline) = stats.latest_video_timeline_observation.as_ref() else {
        return false;
    };
    if has_current_transport_await_issue_from_observation(
        timeline,
        current_clean_anchor_observed_at_ms,
    ) {
        return true;
    }
    let receiver_receiving = stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| is_receiver_state_receiving(Some(obs.receiver_state.as_str())));
    if !receiver_receiving {
        return false;
    }
    let Some(clean_anchor_at_ms) = current_clean_anchor_observed_at_ms else {
        return false;
    };
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            inspection.observed_at_ms >= clean_anchor_at_ms
                && (timeline.observed_at_ms - inspection.observed_at_ms).max(0.0)
                    <= CURRENT_TRANSPORT_AWAIT_INVALID_BOOTSTRAP_FRESH_MS
                && inspection_has_invalid_recovery_bootstrap(inspection)
        })
}

/// 从 timeline 观测推导统一 `GapSeverity`（不含 episode stalled→RecoveryBlocked，见
/// [`derive_gap_severity_with_episode_stall`]）。
pub(crate) fn derive_gap_severity_from_timeline_observation(
    timeline: &XbxEngineVideoTimelineObservation,
) -> GapSeverity {
    let reason = timeline.chain.reason.as_deref();
    if matches!(reason, Some("referenceChainUnrecoverable")) {
        if chain_broken_observation_lacks_media_evidence(timeline) {
            return GapSeverity::ReferenceGap;
        }
        return GapSeverity::ChainBroken;
    }
    if matches!(
        reason,
        Some("awaitingRecoveryAnchor" | "awaitRecoveryAnchor" | "receiverWaitingKeyframe",)
    ) {
        return GapSeverity::AnchorGap;
    }
    if let Some(gap) = timeline.gap.as_ref() {
        if timeline_gap_implies_reference_gap_evidence(gap) {
            return GapSeverity::ReferenceGap;
        }
        return GapSeverity::RepairableGap;
    }
    GapSeverity::LowValueGap
}

/// 纯 transport 预算抬价 + 匿名缺洞时，不把 `chain.reason` 上的坏链语义升级成 `ChainBroken`。
fn chain_broken_observation_lacks_media_evidence(
    timeline: &XbxEngineVideoTimelineObservation,
) -> bool {
    let Some(gap) = timeline.gap.as_ref() else {
        return false;
    };
    if gap.frame_rtp_timestamp.is_some() {
        return false;
    }
    if gap.gap_dependency_confidence.as_deref() == Some("bound") {
        return false;
    }
    let evidence = gap.evidence_importance.as_deref().unwrap_or("unknown");
    if evidence != "unknown" {
        return false;
    }
    matches!(gap.budget_importance.as_deref(), Some("supply" | "anchor"))
}

/// 仅当 gap 快照携带 reference 级媒体/依赖证据时，才允许从「可修补缺口」升格为 `ReferenceGap`。
fn timeline_gap_implies_reference_gap_evidence(gap: &XbxEngineVideoTimelineGapSnapshot) -> bool {
    if gap.gap_dependency_confidence.as_deref() == Some("bound") {
        return true;
    }
    matches!(
        gap.evidence_importance.as_deref(),
        Some("reference" | "supply" | "anchor")
    ) || matches!(
        gap.frame_importance.as_deref(),
        Some("reference" | "supply" | "anchor" | "keyframe")
    )
}

/// 与 keyframe episode stalled（无推进边沿）叠加时，将严重度提升为 `RecoveryBlocked`。
pub(crate) fn derive_gap_severity_with_episode_stall(
    timeline: &XbxEngineVideoTimelineObservation,
    episode_stalled_no_progress: bool,
) -> GapSeverity {
    if episode_stalled_no_progress {
        let reason = timeline.chain.reason.as_deref();
        if matches!(
            reason,
            Some(
                "awaitingRecoveryAnchor"
                    | "awaitRecoveryAnchor"
                    | "receiverWaitingKeyframe"
                    | "referenceChainUnrecoverable",
            )
        ) {
            return GapSeverity::RecoveryBlocked;
        }
    }
    derive_gap_severity_from_timeline_observation(timeline)
}

/// ledger / NACK / owner 共用的 `FrameValue` 映射（`RecoveryBlocked` 不映射帧价值，由调用方保留基线）。
pub(crate) fn frame_value_from_gap_severity(gs: GapSeverity) -> Option<FrameValue> {
    match gs {
        GapSeverity::RecoveryBlocked => None,
        GapSeverity::ChainBroken | GapSeverity::AnchorGap => Some(FrameValue::RecoveryAnchor),
        GapSeverity::ReferenceGap => Some(FrameValue::Reference),
        GapSeverity::LowValueGap | GapSeverity::RepairableGap => Some(FrameValue::Continuity),
    }
}

pub(crate) fn is_media_healthy_baseline(
    connected: bool,
    chain_healthy: bool,
    track_state: Option<&str>,
    track_video_bytes_total: Option<u64>,
    decode_age_ms: Option<f64>,
    present_age_ms: Option<f64>,
    decode_fresh_limit_ms: f64,
    present_fresh_limit_ms: f64,
    decoder_stalled: bool,
    renderer_stalled: bool,
) -> bool {
    if !connected || !chain_healthy || decoder_stalled || renderer_stalled {
        return false;
    }
    let track_attached = matches!(track_state, Some("remoteTrackAttached"));
    let has_video_bytes = track_video_bytes_total.is_some_and(|bytes| bytes > 0);
    let decode_fresh = decode_age_ms.is_some_and(|age| age <= decode_fresh_limit_ms);
    let present_fresh = present_age_ms.is_some_and(|age| age <= present_fresh_limit_ms);
    track_attached && has_video_bytes && decode_fresh && present_fresh
}

/// 对外单一恢复表面相位（收敛 transport-await / sustaining / waiting-keyframe 叙事）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RecoverySurfacePhase {
    #[default]
    Steady,
    Repairing,
    AwaitIdr,
    SupplyBreak,
}

impl RecoverySurfacePhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Steady => "steady",
            Self::Repairing => "repairing",
            Self::AwaitIdr => "await-idr",
            Self::SupplyBreak => "supply-break",
        }
    }
}

/// 全生命周期对外主叙事（Owner / Insert / trace 单轨投影源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum MediaSupplyPhase {
    #[default]
    Priming,
    Steady,
    Repairing,
    MustIdr,
    SupplyBreak,
}

impl MediaSupplyPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Priming => "priming",
            Self::Steady => "steady",
            Self::Repairing => "repairing",
            Self::MustIdr => "must-idr",
            Self::SupplyBreak => "supply-break",
        }
    }
}

/// 首显后 acquisition 窗：decode/submit 分段延迟阈值（与 Owner degraded 同量级，独立于 picture 叙事）。
pub(crate) const MEDIA_SUPPLY_PRIMING_MAX_DECODE_AGE_MS: f64 = 200.0;
pub(crate) const MEDIA_SUPPLY_PRIMING_MAX_SUBMIT_AGE_MS: f64 = 500.0;
/// 首显后固定 acquisition 窗（与 MEDIA_SUPPLY_GATE 起播 5s 对齐）；窗内恒为 `priming` 超相位。
pub(crate) const MEDIA_SUPPLY_PRIMING_ACQUISITION_WINDOW_MS: f64 = 5_000.0;

pub(crate) fn media_supply_acquisition_window_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    stats.host_frame_present_epoch > 0
        && stats
            .media_supply_host_first_present_at_ms
            .is_some_and(|at_ms| {
                (now_ms - at_ms).max(0.0) < MEDIA_SUPPLY_PRIMING_ACQUISITION_WINDOW_MS
            })
}

pub(crate) fn media_supply_decode_age_ms_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> Option<f64> {
    stats
        .latest_video_decode_ok_time_ms
        .map(|ts| (now_ms - ts).max(0.0))
}

/// Priming 完成：首显 + 当拍 decode/submit 均健康。picture `PlaybackRecovered`  alone 不算完成。
pub(crate) fn media_supply_priming_complete_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats.host_frame_present_epoch == 0 {
        return false;
    }
    let decode_ok = media_supply_decode_age_ms_from_stats(stats, now_ms)
        .is_some_and(|age| age <= MEDIA_SUPPLY_PRIMING_MAX_DECODE_AGE_MS);
    let submit_ok = stats
        .submit_age_ms
        .is_some_and(|age| age <= MEDIA_SUPPLY_PRIMING_MAX_SUBMIT_AGE_MS);
    decode_ok && submit_ok
}

/// 起播 acquisition 超相位：无首显 / 首显后 5s 窗 / 分段延迟未健康 → 恒 `priming`，吞 repairing 子态。
pub(crate) fn media_supply_priming_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats.host_frame_present_epoch == 0 {
        return true;
    }
    if media_supply_acquisition_window_active_from_stats(stats, now_ms) {
        return true;
    }
    !media_supply_priming_complete_from_stats(stats, now_ms)
}

pub(crate) fn recovery_effective_rtt_ms_from_stats(stats: &XbxEngineMediaRuntimeStats) -> f64 {
    stats
        .recovery_effective_rtt_ms
        .or(stats.recovery_smoothed_rtt_ms)
        .unwrap_or(80.0)
        .clamp(20.0, 400.0)
}

pub(crate) fn derive_media_supply_phase_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> MediaSupplyPhase {
    let effective_rtt_ms = recovery_effective_rtt_ms_from_stats(stats);
    if recovery_supply_break_active_from_stats(stats, now_ms)
        || timed_fallback_decoder_unstick_required_from_stats(stats, now_ms)
    {
        return MediaSupplyPhase::SupplyBreak;
    }
    if media_supply_priming_from_stats(stats, now_ms) {
        return MediaSupplyPhase::Priming;
    }
    let waiting_keyframe = stats.video_decoder_recovery_state.as_deref()
        == Some("waiting-keyframe")
        || stats
            .latest_video_receiver_observation
            .as_ref()
            .is_some_and(|obs| obs.receiver_state == "waiting-keyframe");
    let ps_strict = parameter_sets_change_strict_active_from_stats(stats, now_ms, effective_rtt_ms);
    let gap_mode = resolve_gap_vs_keyframe_mode(stats, now_ms, effective_rtt_ms);
    let keyframe_only = gap_keyframe_only_mode_active(gap_mode);
    if waiting_keyframe && (ps_strict || keyframe_only) {
        return MediaSupplyPhase::MustIdr;
    }
    let timeline_repairing = stats
        .latest_video_timeline_observation
        .as_ref()
        .is_some_and(|timeline| {
            matches!(
                timeline.chain.state.as_str(),
                "repairing" | "waiting-keyframe"
            ) || timeline.gap.is_some()
        });
    let receiver_repairing = stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| obs.receiver_state == "repairing");
    if timeline_repairing || receiver_repairing {
        return MediaSupplyPhase::Repairing;
    }
    if waiting_keyframe {
        return MediaSupplyPhase::MustIdr;
    }
    MediaSupplyPhase::Steady
}

pub(crate) fn recovery_surface_phase_from_media_supply_phase(
    phase: MediaSupplyPhase,
) -> RecoverySurfacePhase {
    match phase {
        MediaSupplyPhase::SupplyBreak => RecoverySurfacePhase::SupplyBreak,
        MediaSupplyPhase::MustIdr => RecoverySurfacePhase::AwaitIdr,
        MediaSupplyPhase::Repairing => RecoverySurfacePhase::Repairing,
        MediaSupplyPhase::Priming | MediaSupplyPhase::Steady => RecoverySurfacePhase::Steady,
    }
}

/// 由 decode FSM + 供给事实派生，供 owner/facts 读取。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DerivedDecoderHealth {
    #[default]
    Nominal,
    RepairingDecode,
    AwaitIdr,
    SupplyStalled,
}

impl DerivedDecoderHealth {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::RepairingDecode => "repairing-decode",
            Self::AwaitIdr => "await-idr",
            Self::SupplyStalled => "supply-stalled",
        }
    }
}

const RECOVERY_SUPPLY_BREAK_SUBMIT_AGE_MS: f64 = 1_500.0;

/// 单 tick 从 stats 派生的合同快照；Owner/policy/coordinator 应优先读此结构，避免重复调用 6+ 个 helper。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct RecoveryContractSnapshot {
    pub(crate) media_supply_phase: MediaSupplyPhase,
    pub(crate) surface_phase: RecoverySurfacePhase,
    pub(crate) derived_health: DerivedDecoderHealth,
    pub(crate) exit_path: RecoveryExitPath,
    pub(crate) serving_wide: bool,
    pub(crate) serving_relaxed: bool,
    pub(crate) supply_break_active: bool,
}

impl RecoveryContractSnapshot {
    pub(crate) fn from_stats(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
        exit_thresholds: RecoveryExitThresholds,
    ) -> Self {
        let serving_wide = displayed_idr_serving_from_stats(stats);
        let serving_relaxed =
            displayed_idr_serving_allows_relaxed_controls_from_stats(stats, now_ms);
        let supply_break_active = recovery_supply_break_active_from_stats(stats, now_ms);
        let media_supply_phase = derive_media_supply_phase_from_stats(stats, now_ms);
        Self {
            media_supply_phase,
            surface_phase: recovery_surface_phase_from_media_supply_phase(media_supply_phase),
            derived_health: derive_decoder_health_from_stats(stats, now_ms),
            exit_path: recovery_exit_path_from_stats(stats, now_ms, exit_thresholds),
            serving_wide,
            serving_relaxed,
            supply_break_active,
        }
    }
}

pub(crate) fn recovery_supply_break_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    _now_ms: f64,
) -> bool {
    if stats.video_decoder_recovery_state.as_deref() != Some("waiting-keyframe") {
        return false;
    }
    if !(displayed_idr_serving_from_stats(stats)
        || stats.recovery_playback_recovered_at_ms.is_some())
    {
        return false;
    }
    let submit_stalled = stats
        .submit_age_ms
        .is_some_and(|age| age >= RECOVERY_SUPPLY_BREAK_SUBMIT_AGE_MS);
    let present_starved = stats.video_renderer_stalled.unwrap_or(false)
        && stats.display_age_ms.map(|age| age >= 500.0).unwrap_or(true);
    submit_stalled || present_starved
}

pub(crate) fn derive_recovery_surface_phase_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> RecoverySurfacePhase {
    recovery_surface_phase_from_media_supply_phase(derive_media_supply_phase_from_stats(
        stats, now_ms,
    ))
}

pub(crate) fn derive_decoder_health_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> DerivedDecoderHealth {
    if recovery_supply_break_active_from_stats(stats, now_ms) {
        return DerivedDecoderHealth::SupplyStalled;
    }
    match stats.video_decoder_recovery_state.as_deref() {
        Some("waiting-keyframe") => DerivedDecoderHealth::AwaitIdr,
        Some("recovering") => DerivedDecoderHealth::RepairingDecode,
        _ => DerivedDecoderHealth::Nominal,
    }
}

/// owner / suspect_anchor / fast-path 门控：解码器处于等 IDR 或供给断裂派生态。
pub(crate) fn derived_decoder_health_indicates_await_idr_or_supply_stall(
    stats: &XbxEngineMediaRuntimeStats,
) -> bool {
    matches!(
        stats.derived_decoder_health.as_deref(),
        Some("await-idr" | "supply-stalled")
    )
}

pub(crate) fn sync_derived_recovery_contract_fields(
    stats: &mut XbxEngineMediaRuntimeStats,
    now_ms: f64,
) {
    let media = derive_media_supply_phase_from_stats(stats, now_ms);
    let surface = recovery_surface_phase_from_media_supply_phase(media);
    let health = derive_decoder_health_from_stats(stats, now_ms);
    stats.media_supply_phase = Some(media.as_str().to_string());
    stats.recovery_surface_phase = Some(surface.as_str().to_string());
    stats.derived_decoder_health = Some(health.as_str().to_string());
    stats.recovery_transport_await_unresolved = Some(
        stats
            .recovery_diagnosis
            .as_deref()
            .is_some_and(is_transport_await_unresolved_reason)
            && surface != RecoverySurfacePhase::Steady
            && !recovery_supply_break_active_from_stats(stats, now_ms),
    );
}

#[cfg(test)]
mod derive_gap_observation_tests {
    use super::*;
    use crate::{
        XbxEngineH264InspectionObservation, XbxEngineMediaRuntimeStats,
        XbxEngineVideoTimelineChainSnapshot, XbxEngineVideoTimelineGapSnapshot,
        XbxEngineVideoTimelineObservation,
    };

    #[test]
    fn timeline_gap_without_reference_evidence_maps_to_repairable_gap() {
        let obs = XbxEngineVideoTimelineObservation {
            observation_id: 99,
            source_event: "gap-observed".into(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "observed".into(),
                sequence: Some(10),
                frame_rtp_timestamp: Some(42),
                frame_importance: Some("delta".into()),
                budget_importance: Some("disposable".into()),
                evidence_importance: Some("unknown".into()),
                gap_dependency_confidence: Some("anonymous".into()),
                observed_at_ms: 0.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "receiving".into(),
                reason: None,
                chain_break_evidence: None,
                observed_at_ms: 0.0,
            },
            observed_at_ms: 0.0,
        };
        assert_eq!(
            derive_gap_severity_from_timeline_observation(&obs),
            GapSeverity::RepairableGap
        );
    }

    #[test]
    fn chain_broken_reason_with_anonymous_budget_only_gap_maps_to_reference_severity() {
        let obs = XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "t".into(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "observed".into(),
                sequence: Some(1),
                frame_rtp_timestamp: None,
                frame_importance: Some("unknown".into()),
                budget_importance: Some("supply".into()),
                evidence_importance: Some("unknown".into()),
                gap_dependency_confidence: Some("anonymous".into()),
                observed_at_ms: 0.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".into(),
                reason: Some("referenceChainUnrecoverable".into()),
                chain_break_evidence: None,
                observed_at_ms: 0.0,
            },
            observed_at_ms: 0.0,
        };
        assert_eq!(
            derive_gap_severity_from_timeline_observation(&obs),
            GapSeverity::ReferenceGap
        );
    }

    #[test]
    fn fresh_invalid_bootstrap_breaks_sustaining_recovery_suppression_after_clean_anchor() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 7;
        stats.video_anchor_clean_epoch = Some(7);
        stats.video_anchor_clean_observed_at_ms = Some(100.0);
        stats.video_anchor_clean_source_event = Some("displayed-idr".into());
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
        stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-complete-candidate-decode-feedback-blocked".into(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "expired".into(),
                sequence: Some(1),
                frame_rtp_timestamp: None,
                frame_importance: Some("anchor".into()),
                budget_importance: Some("disposable".into()),
                evidence_importance: Some("anchor".into()),
                gap_dependency_confidence: Some("anonymous".into()),
                observed_at_ms: 180.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "sustaining-recovery".into(),
                reason: Some("recoverySustaining".into()),
                chain_break_evidence: None,
                observed_at_ms: 180.0,
            },
            observed_at_ms: 180.0,
        });
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            observation_id: 2,
            frame_rtp_timestamp: Some(7001),
            nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".into()],
            nal_count: 1,
            vcl_nal_count: 1,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: Some(1920),
            sample_height: Some(1080),
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("NonIdrVcl".into()),
            admission_accepted: true,
            observed_at_ms: 190.0,
            ..Default::default()
        });

        assert!(!has_current_transport_await_issue_from_stats(&stats));
    }

    #[test]
    fn stale_invalid_bootstrap_does_not_break_sustaining_recovery_suppression() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 7;
        stats.video_anchor_clean_epoch = Some(7);
        stats.video_anchor_clean_observed_at_ms = Some(100.0);
        stats.video_anchor_clean_source_event = Some("displayed-idr".into());
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
        stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-complete-candidate-decode-feedback-blocked".into(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "expired".into(),
                sequence: Some(1),
                frame_rtp_timestamp: None,
                frame_importance: Some("anchor".into()),
                budget_importance: Some("disposable".into()),
                evidence_importance: Some("anchor".into()),
                gap_dependency_confidence: Some("anonymous".into()),
                observed_at_ms: 500.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "sustaining-recovery".into(),
                reason: Some("recoverySustaining".into()),
                chain_break_evidence: None,
                observed_at_ms: 500.0,
            },
            observed_at_ms: 500.0,
        });
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            observation_id: 2,
            frame_rtp_timestamp: Some(7001),
            nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".into()],
            nal_count: 1,
            vcl_nal_count: 1,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: Some(1920),
            sample_height: Some(1080),
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("NonIdrVcl".into()),
            admission_accepted: true,
            observed_at_ms: 190.0,
            ..Default::default()
        });

        assert!(!has_current_transport_await_issue_from_stats(&stats));
    }

    #[test]
    fn recovery_progress_level_mapping_follows_rfc_order() {
        assert_eq!(
            recovery_progress_level_from_episode(
                "requested",
                Some("pending"),
                None,
                None,
                None,
                false,
                false
            ),
            Some(RecoveryProgressLevel::WaitingResponse)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "response-observed",
                Some("on-time"),
                Some(false),
                None,
                None,
                false,
                false
            ),
            Some(RecoveryProgressLevel::ContinuationSeen)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "packet-seen",
                Some("on-time"),
                Some(true),
                Some(10.0),
                None,
                false,
                false
            ),
            Some(RecoveryProgressLevel::AnchorSeen)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "decoded",
                Some("on-time"),
                Some(true),
                Some(10.0),
                Some(20.0),
                false,
                false
            ),
            Some(RecoveryProgressLevel::Decoded)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "decoded",
                Some("cleanAnchorCommitted"),
                Some(true),
                Some(10.0),
                Some(20.0),
                true,
                false
            ),
            Some(RecoveryProgressLevel::CleanAnchorCommitted)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "decoded",
                Some("cleanAnchorCommitted"),
                Some(true),
                Some(10.0),
                Some(20.0),
                true,
                true
            ),
            Some(RecoveryProgressLevel::DisplayStable)
        );
    }

    #[test]
    fn recovery_progress_gap_helpers_match_contract() {
        assert!(recovery_progress_missing_anchor(Some(
            RecoveryProgressLevel::WaitingResponse
        )));
        assert!(recovery_progress_missing_anchor(Some(
            RecoveryProgressLevel::ContinuationSeen
        )));
        assert!(!recovery_progress_missing_anchor(Some(
            RecoveryProgressLevel::AnchorSeen
        )));
        assert_eq!(
            recovery_progress_level_from_str("ContinuationSeen"),
            Some(RecoveryProgressLevel::ContinuationSeen)
        );
        assert_eq!(recovery_progress_level_from_str("unknown"), None);
        assert!(recovery_progress_allows_decoder_reset(Some(
            RecoveryProgressLevel::Decoded
        )));
        assert!(!recovery_progress_allows_decoder_reset(Some(
            RecoveryProgressLevel::ContinuationSeen
        )));
    }

    #[test]
    fn recovery_display_facts_projects_from_stats() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_rtp = Some(77_001);
        stats.recovery_displayed_idr_at_ms = Some(120.0);
        stats.recovery_playback_recovered_at_ms = Some(130.0);
        stats.recovery_fresh_anchor_recovered_at_ms = Some(120.0);
        let display = RecoveryDisplayFacts::from_stats(&stats);
        assert_eq!(display.displayed_idr_rtp, Some(77_001));
        assert!(display.has_established_displayed_idr());
        assert!(has_current_clean_anchor_from_stats(&stats));
    }

    #[test]
    fn transport_await_hard_bootstrap_evidence_uses_non_idr_reject() {
        let now_ms = 2_000.0;
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 41,
            latest_video_timeline_observation: Some(XbxEngineVideoTimelineObservation {
                observation_id: 1,
                source_event: "frame-await-recovery-anchor".into(),
                gap: None,
                frame: None,
                chain: XbxEngineVideoTimelineChainSnapshot {
                    state: "recovering".into(),
                    reason: Some("receiverWaitingKeyframe".into()),
                    chain_break_evidence: None,
                    observed_at_ms: now_ms - 8.0,
                },
                observed_at_ms: now_ms - 8.0,
            }),
            latest_h264_inspection_observation: Some(XbxEngineH264InspectionObservation {
                observation_id: 2,
                frame_rtp_timestamp: Some(3_333),
                nal_types: vec![],
                nal_count: 0,
                vcl_nal_count: 0,
                has_inband_sps: false,
                has_inband_pps: false,
                committed_sps_present: true,
                committed_pps_present: true,
                slice_headers_valid: true,
                delta_continuation_ready: true,
                parameter_sets_changed: false,
                config_changed: false,
                is_idr: false,
                sample_width: None,
                sample_height: None,
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("NonIdrVcl".into()),
                admission_accepted: true,
                observed_at_ms: now_ms - 6.0,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(transport_await_has_hard_bootstrap_evidence_from_stats(
            &stats, now_ms
        ));
    }

    #[test]
    fn displayed_idr_serving_true_when_pending_idr_and_host_has_presented() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_pending_displayed_idr_rtp = Some(77_001);
        stats.host_frame_present_epoch = 1;
        assert!(displayed_idr_serving_from_stats(&stats));
        assert_eq!(
            resolve_host_display_idr_anchor_rtp(&stats, Some(77_002)),
            Some(77_001)
        );
    }

    #[test]
    fn displayed_idr_serving_false_without_host_present_epoch() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_pending_displayed_idr_rtp = Some(77_001);
        stats.host_frame_present_epoch = 0;
        assert!(!displayed_idr_serving_from_stats(&stats));
    }

    #[test]
    fn collapse_waiting_keyframe_when_displayed_idr_serving_with_gap() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        assert!(should_collapse_receiver_waiting_keyframe_to_repairing(
            &stats, 200.0, true, 10
        ));
        assert!(!should_collapse_receiver_waiting_keyframe_to_repairing(
            &stats, 200.0, false, 10
        ));
    }

    #[test]
    fn collapse_enabled_when_clean_anchor_and_decoder_waiting_keyframe() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        assert!(should_collapse_receiver_waiting_keyframe_to_repairing(
            &stats, 200.0, true, 10
        ));
    }

    #[test]
    fn relaxation_unblocked_under_supply_break_when_submit_stale() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.recovery_playback_recovered_at_ms = Some(100.0);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.submit_age_ms = Some(1_500.0);
        stats.video_renderer_stalled = Some(true);
        assert!(!displayed_idr_serving_relaxation_blocked_from_stats(
            &stats, 200.0
        ));
        assert!(displayed_idr_serving_allows_relaxed_controls_from_stats(
            &stats, 200.0
        ));
    }

    #[test]
    fn relaxation_not_blocked_by_soft_bootstrap_missing_idr_when_displayed_idr_serving() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            observed_at_ms: 180.0,
            admission_accepted: false,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            ..Default::default()
        });
        assert!(!transport_await_has_hard_bootstrap_evidence_from_stats(
            &stats, 200.0
        ));
        assert!(displayed_idr_serving_allows_relaxed_controls_from_stats(
            &stats, 200.0
        ));
    }

    #[test]
    fn waiting_keyframe_without_idr_progress_blocks_decoder_reset() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        assert!(!decoder_reset_permitted_from_stats(
            &stats, None, 1_000.0, false
        ));
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            observed_at_ms: 990.0,
            is_idr: true,
            admission_accepted: true,
            ..Default::default()
        });
        assert!(decoder_reset_permitted_from_stats(
            &stats, None, 1_000.0, false
        ));
    }

    #[test]
    fn clean_anchor_keeps_displayed_idr_relaxation_under_waiting_keyframe() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        assert!(!displayed_idr_serving_relaxation_blocked_from_stats(
            &stats, 5_000.0
        ));
        assert!(displayed_idr_serving_allows_relaxed_controls_from_stats(
            &stats, 5_000.0
        ));
    }

    #[test]
    fn supply_break_when_waiting_keyframe_and_submit_stalled() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.submit_age_ms = Some(5_000.0);
        stats.recovery_playback_recovered_at_ms = Some(1.0);
        assert!(recovery_supply_break_active_from_stats(&stats, 10_000.0));
        let snap = RecoveryContractSnapshot::from_stats(
            &stats,
            10_000.0,
            RecoveryExitThresholds::default(),
        );
        assert!(snap.supply_break_active);
        assert_eq!(snap.surface_phase, RecoverySurfacePhase::SupplyBreak);
        assert_eq!(
            derive_recovery_surface_phase_from_stats(&stats, 10_000.0),
            RecoverySurfacePhase::SupplyBreak
        );
        assert_eq!(
            derive_decoder_health_from_stats(&stats, 10_000.0),
            DerivedDecoderHealth::SupplyStalled
        );
    }

    #[test]
    fn recovery_exit_timed_fallback_when_submit_stalled_without_anchor() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.submit_age_ms = Some(2_000.0);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        assert_eq!(
            recovery_exit_path_from_stats(&stats, 5_000.0, RecoveryExitThresholds::default()),
            RecoveryExitPath::TimedFallback
        );
    }

    #[test]
    fn displayed_idr_relaxation_unblocked_under_timed_fallback() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.submit_age_ms = Some(2_000.0);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        assert!(!displayed_idr_serving_relaxation_blocked_from_stats(
            &stats, 5_000.0
        ));
    }

    #[test]
    fn timed_fallback_decoder_unstick_when_submit_and_decode_stale() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.submit_age_ms = Some(5_000.0);
        stats.latest_video_decode_ok_time_ms = Some(1_000.0);
        stats.video_decoder_stalled = Some(true);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        assert!(timed_fallback_decoder_unstick_required_from_stats(
            &stats, 10_000.0
        ));
        assert!(decoder_reset_permitted_from_stats(
            &stats,
            Some(RecoveryProgressLevel::WaitingResponse),
            10_000.0,
            false,
        ));
    }

    #[test]
    fn stale_submit_break_disabled_under_timed_fallback() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.submit_age_ms = Some(10_000.0);
        stats.video_decoder_stalled = Some(true);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        assert!(!displayed_idr_serving_relaxation_blocked_from_stats(
            &stats, 20_000.0
        ));
    }

    #[test]
    fn recovery_exit_timed_fallback_over_stale_displayed_idr_fact() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.recovery_playback_recovered_at_ms = Some(100.0);
        stats.latest_video_decode_ok_time_ms = Some(4_900.0);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.submit_age_ms = Some(2_000.0);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        assert_eq!(
            recovery_exit_path_from_stats(&stats, 5_000.0, RecoveryExitThresholds::default()),
            RecoveryExitPath::TimedFallback
        );
    }

    #[test]
    fn recovery_exit_decode_output_when_decode_and_host_output_fresh() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_video_decode_ok_time_ms = Some(4_900.0);
        stats.host_frame_present_epoch = 3;
        stats.recovery_playback_recovered_at_ms = Some(100.0);
        stats.submit_age_ms = Some(400.0);
        assert_eq!(
            recovery_exit_path_from_stats(&stats, 5_000.0, RecoveryExitThresholds::default()),
            RecoveryExitPath::DecodeOutput
        );
    }

    #[test]
    fn parameter_sets_change_strict_active_within_window() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(500.0);
        stats.host_frame_present_epoch = 1;
        stats.recovery_playback_recovered_at_ms = Some(600.0);
        stats.latest_video_decode_ok_time_ms = Some(1_250.0);
        stats.submit_age_ms = Some(80.0);
        stats.video_parameter_sets_changed_at_ms = Some(1_000.0);
        assert!(parameter_sets_change_strict_active_from_stats(
            &stats, 1_300.0, 50.0
        ));
        assert!(!parameter_sets_change_strict_active_from_stats(
            &stats, 1_500.0, 50.0
        ));
    }

    #[test]
    fn parameter_sets_change_strict_inactive_during_priming() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_parameter_sets_changed_at_ms = Some(1_000.0);
        stats.host_frame_present_epoch = 1;
        assert!(!parameter_sets_change_strict_active_from_stats(
            &stats, 1_200.0, 50.0
        ));
        assert_eq!(
            derive_media_supply_phase_from_stats(&stats, 1_200.0),
            MediaSupplyPhase::Priming
        );
    }

    #[test]
    fn media_supply_priming_absorbs_repairing_until_segment_ages_healthy() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.host_frame_present_epoch = 68;
        stats.media_supply_host_first_present_at_ms = Some(1_900.0);
        stats.recovery_displayed_idr_at_ms = Some(1_000.0);
        stats.recovery_playback_recovered_at_ms = Some(1_500.0);
        stats.latest_video_decode_ok_time_ms = Some(1_950.0);
        stats.submit_age_ms = Some(800.0);
        stats.latest_video_receiver_observation = Some(crate::XbxEngineVideoReceiverObservation {
            observation_id: 1,
            receiver_state: "repairing".to_string(),
            gap_sequence: Some(10),
            gap_span: Some(2),
            nack_in_flight: true,
            keyframe_request_pending: false,
            bootstrap_reject_reason: None,
            observed_at_ms: 1_900.0,
        });
        stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "gap".to_string(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "pending".to_string(),
                sequence: Some(10),
                frame_rtp_timestamp: None,
                frame_importance: None,
                budget_importance: None,
                evidence_importance: None,
                gap_dependency_confidence: None,
                observed_at_ms: 1_900.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "repairing".to_string(),
                reason: None,
                chain_break_evidence: None,
                observed_at_ms: 1_900.0,
            },
            observed_at_ms: 1_900.0,
        });
        assert_eq!(
            derive_media_supply_phase_from_stats(&stats, 2_000.0),
            MediaSupplyPhase::Priming
        );
        stats.submit_age_ms = Some(120.0);
        assert_eq!(
            derive_media_supply_phase_from_stats(&stats, 2_000.0),
            MediaSupplyPhase::Priming
        );
        stats.latest_video_decode_ok_time_ms = Some(6_950.0);
        assert_eq!(
            derive_media_supply_phase_from_stats(&stats, 7_000.0),
            MediaSupplyPhase::Repairing
        );
    }

    #[test]
    fn media_supply_acquisition_window_forces_priming_despite_healthy_segment_ages() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.host_frame_present_epoch = 84;
        stats.media_supply_host_first_present_at_ms = Some(46_000.0);
        stats.latest_video_decode_ok_time_ms = Some(46_020.0);
        stats.submit_age_ms = Some(29.0);
        stats.latest_video_receiver_observation = Some(crate::XbxEngineVideoReceiverObservation {
            observation_id: 1,
            receiver_state: "repairing".to_string(),
            gap_sequence: Some(1),
            gap_span: None,
            nack_in_flight: true,
            keyframe_request_pending: true,
            bootstrap_reject_reason: None,
            observed_at_ms: 46_100.0,
        });
        assert_eq!(
            derive_media_supply_phase_from_stats(&stats, 46_200.0),
            MediaSupplyPhase::Priming
        );
    }

    #[test]
    fn media_supply_must_idr_when_waiting_with_ps_strict() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.host_frame_present_epoch = 2;
        stats.recovery_playback_recovered_at_ms = Some(100.0);
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.latest_video_decode_ok_time_ms = Some(950.0);
        stats.submit_age_ms = Some(80.0);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.video_parameter_sets_changed_at_ms = Some(900.0);
        assert_eq!(
            derive_media_supply_phase_from_stats(&stats, 1_000.0),
            MediaSupplyPhase::MustIdr
        );
        assert_eq!(
            recovery_surface_phase_from_media_supply_phase(MediaSupplyPhase::MustIdr),
            RecoverySurfacePhase::AwaitIdr
        );
    }

    #[test]
    fn repairing_missing_idr_pressure_forces_keyframe_only_mode() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.host_frame_present_epoch = 1;
        stats.recovery_playback_recovered_at_ms = Some(200.0);
        stats.latest_video_receiver_observation = Some(crate::XbxEngineVideoReceiverObservation {
            observation_id: 1,
            receiver_state: "repairing".to_string(),
            gap_sequence: Some(10),
            gap_span: Some(2),
            nack_in_flight: true,
            keyframe_request_pending: false,
            bootstrap_reject_reason: None,
            observed_at_ms: 900.0,
        });
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            observed_at_ms: 950.0,
            is_idr: false,
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            ..Default::default()
        });
        stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "gap".to_string(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "pending".to_string(),
                sequence: Some(10),
                frame_rtp_timestamp: None,
                frame_importance: None,
                budget_importance: None,
                evidence_importance: None,
                gap_dependency_confidence: None,
                observed_at_ms: 900.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "receiving".to_string(),
                reason: None,
                chain_break_evidence: None,
                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
        assert_eq!(
            resolve_gap_vs_keyframe_mode(&stats, 1_000.0, 50.0),
            GapVsKeyframeMode::KeyframeOnly
        );
    }

    #[test]
    fn relaxation_still_blocked_by_invalid_bootstrap_when_displayed_idr_serving() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            observed_at_ms: 180.0,
            admission_accepted: false,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
            ..Default::default()
        });
        assert!(transport_await_has_hard_bootstrap_evidence_from_stats(
            &stats, 200.0
        ));
        assert!(!displayed_idr_serving_allows_relaxed_controls_from_stats(
            &stats, 200.0
        ));
    }
}
