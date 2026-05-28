//! Pre-decode 单点裁决：对齐 libwebrtc Insert 纪律（Emit / HoldRepair / DropCorrupt）。

use crate::media::video::h264::inspection::H264AccessUnitInspection;
use crate::transport::rtc::receive::decode_gate::{
    insert_decodable_to_feed, should_block_non_keyframe_admission, DecodeCorruptionPolicy,
    InspectionAdmission, ReceiverDecodeContext,
};
use crate::transport::rtc::recovery::contract::{
    derive_media_supply_phase_from_stats, derive_packet_recovery_action_stage_from_stats,
    fresh_h264_idr_admission_from_stats, gap_keyframe_only_mode_active,
    media_supply_submit_starved_from_stats, parameter_sets_change_strict_active_from_stats,
    recovery_supply_break_active_from_stats, resolve_gap_vs_keyframe_mode, GapVsKeyframeMode,
    MediaSupplyPhase, PacketRecoveryActionStage,
};
use crate::XbxEngineMediaRuntimeStats;

/// 与 libwebrtc Insert 对齐的 pre-decode 裁决。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InsertDecision {
    /// 进入解码器（可接受花屏 continuation）。
    Emit,
    /// 不进解码器；允许 NACK/修洞继续。
    HoldRepair,
    /// 丢弃并触发 receive 侧 PLI/FIR 路径。
    DropCorrupt,
}

/// Insert 裁决原因（收敛 trace / stats 标签）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InsertDecisionReason {
    CorruptIdrWithLoss,
    FreshIdr,
    PostPsStrict,
    MustIdrHold,
    DecodableToFeed,
    FirstFrameFreshOrBootstrapIdr,
    FirstFrameHold,
    HoldDefault,
}

impl InsertDecisionReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CorruptIdrWithLoss => "corruptIdrWithLoss",
            Self::FreshIdr => "freshIdr",
            Self::PostPsStrict => "postPsStrict",
            Self::MustIdrHold => "mustIdrHold",
            Self::DecodableToFeed => "decodableToFeed",
            Self::FirstFrameFreshOrBootstrapIdr => "firstFrameFreshOrBootstrapIdr",
            Self::FirstFrameHold => "firstFrameHold",
            Self::HoldDefault => "holdDefault",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct InsertContext {
    pub decode: ReceiverDecodeContext,
    /// 与 `gap_keyframe_only_mode_active(gap_mode)` 联动；`MustIdr` 由 supply 层 `derive_media_supply_phase` 统一裁决。
    pub gap_mode: GapVsKeyframeMode,
    pub action_stage: PacketRecoveryActionStage,
    pub fresh_idr_admission: bool,
    /// PS/config 变更后短窗：非 IDR 一律 HoldRepair，优先 RTCP IDR。
    pub post_parameter_sets_change_strict: bool,
    /// supply-break 续播窄路径：仍允许 repairing delta Emit。
    pub supply_break_continuation: bool,
    pub media_supply_phase: MediaSupplyPhase,
}

impl InsertContext {
    pub(crate) fn from_runtime(
        decode: ReceiverDecodeContext,
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
        effective_rtt_ms: f64,
    ) -> Self {
        Self {
            decode,
            gap_mode: resolve_gap_vs_keyframe_mode(stats, now_ms, effective_rtt_ms),
            action_stage: derive_packet_recovery_action_stage_from_stats(
                stats,
                now_ms,
                effective_rtt_ms,
            ),
            fresh_idr_admission: fresh_h264_idr_admission_from_stats(stats, now_ms),
            post_parameter_sets_change_strict: parameter_sets_change_strict_active_from_stats(
                stats,
                now_ms,
                effective_rtt_ms,
            ),
            supply_break_continuation: recovery_supply_break_active_from_stats(stats, now_ms)
                && !media_supply_submit_starved_from_stats(stats, now_ms),
            media_supply_phase: derive_media_supply_phase_from_stats(stats, now_ms),
        }
    }
}

pub(crate) fn resolve_insert_decision(
    inspection: &H264AccessUnitInspection,
    ctx: &InsertContext,
    policy: DecodeCorruptionPolicy,
    media_dropped_packets: u16,
) -> InsertDecision {
    resolve_insert_decision_with_reason(inspection, ctx, policy, media_dropped_packets).0
}

pub(crate) fn resolve_insert_decision_with_reason(
    inspection: &H264AccessUnitInspection,
    ctx: &InsertContext,
    _policy: DecodeCorruptionPolicy,
    media_dropped_packets: u16,
) -> (InsertDecision, &'static str) {
    let (decision, reason) =
        resolve_insert_decision_with_reason_enum(inspection, ctx, media_dropped_packets);
    (decision, reason.as_str())
}

pub(crate) fn resolve_insert_decision_with_reason_enum(
    inspection: &H264AccessUnitInspection,
    ctx: &InsertContext,
    media_dropped_packets: u16,
) -> (InsertDecision, InsertDecisionReason) {
    if inspection.is_idr && media_dropped_packets > 0 {
        return (
            InsertDecision::DropCorrupt,
            InsertDecisionReason::CorruptIdrWithLoss,
        );
    }
    if ctx.fresh_idr_admission && inspection.is_idr {
        return (InsertDecision::Emit, InsertDecisionReason::FreshIdr);
    }
    if ctx.post_parameter_sets_change_strict && !ctx.supply_break_continuation && !inspection.is_idr
    {
        return (
            InsertDecision::HoldRepair,
            InsertDecisionReason::PostPsStrict,
        );
    }
    if matches!(ctx.media_supply_phase, MediaSupplyPhase::MustIdr)
        && ctx.decode.first_frame_acquired
    {
        if inspection.is_idr {
            if !(inspection.bootstrap_ready || ctx.fresh_idr_admission) {
                return (
                    InsertDecision::HoldRepair,
                    InsertDecisionReason::MustIdrHold,
                );
            }
        } else {
            return (
                InsertDecision::HoldRepair,
                InsertDecisionReason::MustIdrHold,
            );
        }
    }
    if insert_decodable_to_feed(inspection, &ctx.decode, ctx.action_stage) {
        return (InsertDecision::Emit, InsertDecisionReason::DecodableToFeed);
    }
    if inspection.is_idr && inspection.bootstrap_ready {
        return (InsertDecision::Emit, InsertDecisionReason::DecodableToFeed);
    }
    if should_block_non_keyframe_admission(&ctx.decode) && !inspection.is_idr {
        return (
            InsertDecision::HoldRepair,
            InsertDecisionReason::MustIdrHold,
        );
    }
    if !ctx.decode.first_frame_acquired {
        if inspection.is_idr && (ctx.fresh_idr_admission || inspection.bootstrap_ready) {
            return (
                InsertDecision::Emit,
                InsertDecisionReason::FirstFrameFreshOrBootstrapIdr,
            );
        }
        return (
            InsertDecision::HoldRepair,
            InsertDecisionReason::FirstFrameHold,
        );
    }
    (
        InsertDecision::HoldRepair,
        InsertDecisionReason::HoldDefault,
    )
}

pub(crate) fn insert_decision_label(decision: InsertDecision) -> &'static str {
    match decision {
        InsertDecision::Emit => "emit",
        InsertDecision::HoldRepair => "holdRepair",
        InsertDecision::DropCorrupt => "dropCorrupt",
    }
}

/// InsertGate 已 Emit 且 AU 无 bootstrap_ready：decode 侧 bootstrap 闸须放行（与 libwebrtc Insert 对齐）。
pub(crate) fn insert_emit_permits_decode_without_bootstrap_ready(
    inspection: &H264AccessUnitInspection,
    ctx: &InsertContext,
    policy: DecodeCorruptionPolicy,
) -> bool {
    !inspection.bootstrap_ready
        && resolve_insert_decision(inspection, ctx, policy, 0) == InsertDecision::Emit
}

pub(crate) fn recovery_keyframe_action_for_insert_decision(
    decision: InsertDecision,
) -> crate::transport::rtc::receive::decode_gate_eval::RecoveryKeyframeAction {
    use crate::transport::rtc::receive::decode_gate_eval::RecoveryKeyframeAction;
    match decision {
        InsertDecision::Emit => RecoveryKeyframeAction::Submit,
        InsertDecision::HoldRepair => RecoveryKeyframeAction::WaitKeyframe,
        InsertDecision::DropCorrupt => RecoveryKeyframeAction::DropAndRequestPli,
    }
}

pub(crate) fn insert_decision_to_inspection_admission(
    decision: InsertDecision,
) -> InspectionAdmission {
    match decision {
        InsertDecision::Emit => InspectionAdmission::Accept,
        InsertDecision::HoldRepair | InsertDecision::DropCorrupt => {
            InspectionAdmission::AwaitRecoveryKeyframe
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::video::h264::inspection::H264AccessUnitInspector;
    use crate::media::video::test_fixtures::{
        bootstrap_idr_nalu, bootstrap_pps_nalu, bootstrap_sps_nalu,
    };
    use crate::transport::rtc::receive::ReceiverState;

    fn bootstrap_non_idr_nalu() -> Vec<u8> {
        let mut nalu = bootstrap_idr_nalu().to_vec();
        nalu[0] = 0x41;
        nalu
    }

    fn ctx_decoder_synced() -> InsertContext {
        InsertContext {
            decode: ReceiverDecodeContext {
                receiver_state: ReceiverState::Repairing,
                has_active_gap: true,
                nack_exhausted: false,
                first_frame_acquired: true,
                prior_output_established: true,
                displayed_idr_serving: true,
                displayed_idr_decoder_synced: true,
                decoder_reference_synced: true,
            },
            gap_mode: GapVsKeyframeMode::RepairFirst,
            action_stage: PacketRecoveryActionStage::NackPending,
            fresh_idr_admission: false,
            post_parameter_sets_change_strict: false,
            supply_break_continuation: false,
            media_supply_phase: MediaSupplyPhase::Steady,
        }
    }

    fn non_idr_inspection() -> H264AccessUnitInspection {
        let inspector = H264AccessUnitInspector::new();
        inspector
            .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
            .expect("seed sps/pps");
        let mut payload = vec![0, 0, 0, 1];
        payload.extend_from_slice(bootstrap_non_idr_nalu().as_slice());
        inspector
            .inspect_access_unit(&payload)
            .expect("inspect non-idr")
    }

    #[test]
    fn decoder_synced_delta_emits_despite_bootstrap_missing_idr() {
        let ctx = ctx_decoder_synced();
        let inspection = non_idr_inspection();
        assert!(!inspection.bootstrap_ready);
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::Emit
        );
    }

    fn ctx_first_frame_unsynced() -> InsertContext {
        InsertContext {
            decode: ReceiverDecodeContext {
                receiver_state: ReceiverState::WaitingKeyframe,
                has_active_gap: false,
                nack_exhausted: false,
                first_frame_acquired: false,
                prior_output_established: false,
                displayed_idr_serving: false,
                displayed_idr_decoder_synced: false,
                decoder_reference_synced: false,
            },
            gap_mode: GapVsKeyframeMode::RepairFirst,
            action_stage: PacketRecoveryActionStage::WaitKeyframe,
            fresh_idr_admission: false,
            post_parameter_sets_change_strict: false,
            supply_break_continuation: false,
            media_supply_phase: MediaSupplyPhase::MustIdr,
        }
    }

    #[test]
    fn first_frame_emits_only_fresh_or_bootstrap_idr() {
        let inspector = H264AccessUnitInspector::new();
        inspector
            .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
            .expect("seed sps/pps");
        let mut idr_payload = vec![0, 0, 0, 1];
        idr_payload.extend_from_slice(bootstrap_idr_nalu());
        let idr = inspector
            .inspect_access_unit(&idr_payload)
            .expect("inspect idr");
        let ctx = ctx_first_frame_unsynced();
        let (decision, reason) = resolve_insert_decision_with_reason(
            &idr,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );
        assert_eq!(decision, InsertDecision::Emit);
        assert!(
            reason == "decodableToFeed" || reason == "firstFrameFreshOrBootstrapIdr",
            "bootstrap IDR emits via decodable or first-frame path, got {reason}"
        );

        let mut non_bootstrap = idr;
        non_bootstrap.bootstrap_ready = false;
        let (hold, hold_reason) = resolve_insert_decision_with_reason(
            &non_bootstrap,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );
        assert_eq!(hold, InsertDecision::HoldRepair);
        assert_eq!(hold_reason, "firstFrameHold");

        let mut fresh_ctx = ctx;
        fresh_ctx.fresh_idr_admission = true;
        let (fresh_emit, fresh_reason) = resolve_insert_decision_with_reason(
            &non_bootstrap,
            &fresh_ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );
        assert_eq!(fresh_emit, InsertDecision::Emit);
        assert_eq!(fresh_reason, "freshIdr");
    }

    #[test]
    fn must_idr_holds_non_bootstrap_delta_after_first_frame() {
        let mut ctx = ctx_decoder_synced();
        ctx.media_supply_phase = MediaSupplyPhase::MustIdr;
        ctx.decode.receiver_state = ReceiverState::WaitingKeyframe;
        let inspection = non_idr_inspection();
        let (decision, reason) = resolve_insert_decision_with_reason(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );
        assert_eq!(decision, InsertDecision::HoldRepair);
        assert_eq!(reason, "mustIdrHold");
    }

    #[test]
    fn host_displayed_only_without_decoder_sync_holds_non_idr() {
        let mut ctx = ctx_decoder_synced();
        ctx.decode.decoder_reference_synced = false;
        ctx.decode.displayed_idr_decoder_synced = false;
        ctx.decode.displayed_idr_serving = true;
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::HoldRepair
        );
    }

    #[test]
    fn insert_emit_bypass_matches_emit_decision_for_synced_soft_missing_idr() {
        let ctx = ctx_decoder_synced();
        let inspection = non_idr_inspection();
        assert!(insert_emit_permits_decode_without_bootstrap_ready(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
        ));
    }

    #[test]
    fn keyframe_only_mode_holds_non_idr_without_decoder_sync() {
        let mut ctx = ctx_decoder_synced();
        ctx.gap_mode = GapVsKeyframeMode::KeyframeOnly;
        ctx.decode.decoder_reference_synced = false;
        ctx.decode.displayed_idr_decoder_synced = false;
        ctx.decode.displayed_idr_serving = false;
        ctx.action_stage = PacketRecoveryActionStage::WaitKeyframe;
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::HoldRepair
        );
    }

    #[test]
    fn keyframe_only_mode_holds_displayed_idr_soft_missing_idr_without_decoder_sync() {
        let mut ctx = ctx_decoder_synced();
        ctx.gap_mode = GapVsKeyframeMode::KeyframeOnly;
        ctx.decode.decoder_reference_synced = false;
        ctx.decode.displayed_idr_decoder_synced = false;
        ctx.action_stage = PacketRecoveryActionStage::WaitKeyframe;
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::HoldRepair
        );
    }

    #[test]
    fn post_parameter_sets_change_strict_holds_non_idr() {
        let mut ctx = ctx_decoder_synced();
        ctx.post_parameter_sets_change_strict = true;
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::HoldRepair
        );
    }

    #[test]
    fn submit_starved_stats_context_holds_non_idr_even_when_decoder_synced() {
        use crate::transport::rtc::receive::decode_gate::receiver_decode_context_from_stats;
        use crate::XbxEngineMediaRuntimeStats;
        use crate::XbxEngineVideoReceiverObservation;

        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decoder_recovery_state = Some("nominal".to_string());
        stats.recovery_playback_recovered_at_ms = Some(9_900.0);
        stats.recovery_displayed_idr_at_ms = Some(9_950.0);
        stats.recovery_decoder_reference_synced_at_ms = Some(9_950.0);
        stats.latest_video_decode_ok_time_ms = Some(9_950.0);
        stats.latest_video_decode_ok_rtp_timestamp = Some(77_001);
        stats.recovery_displayed_idr_rtp = Some(77_001);
        stats.submit_age_ms = Some(5_000.0);
        stats.latest_video_receiver_observation = Some(XbxEngineVideoReceiverObservation {
            observation_id: 1,
            receiver_state: "repairing".to_string(),
            gap_sequence: Some(1),
            gap_span: Some(1),
            nack_in_flight: true,
            keyframe_request_pending: false,
            bootstrap_reject_reason: None,
            observed_at_ms: 9_950.0,
        });
        let decode = receiver_decode_context_from_stats(&stats, 10_000.0);
        let ctx = InsertContext::from_runtime(decode, &stats, 10_000.0, 50.0);
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::HoldRepair
        );
    }

    #[test]
    fn supply_break_without_submit_starve_may_emit_repairing_delta() {
        use crate::transport::rtc::receive::decode_gate::receiver_decode_context_from_stats;
        use crate::XbxEngineMediaRuntimeStats;
        use crate::XbxEngineVideoReceiverObservation;

        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decoder_recovery_state = Some("nominal".to_string());
        stats.recovery_playback_recovered_at_ms = Some(9_900.0);
        stats.recovery_displayed_idr_at_ms = Some(9_950.0);
        stats.recovery_decoder_reference_synced_at_ms = Some(9_950.0);
        stats.latest_video_decode_ok_time_ms = Some(9_950.0);
        stats.latest_video_decode_ok_rtp_timestamp = Some(77_001);
        stats.recovery_displayed_idr_rtp = Some(77_001);
        stats.submit_age_ms = Some(400.0);
        stats.video_renderer_stalled = Some(true);
        stats.display_age_ms = Some(600.0);
        stats.latest_video_receiver_observation = Some(XbxEngineVideoReceiverObservation {
            observation_id: 1,
            receiver_state: "repairing".to_string(),
            gap_sequence: Some(1),
            gap_span: Some(1),
            nack_in_flight: true,
            keyframe_request_pending: false,
            bootstrap_reject_reason: None,
            observed_at_ms: 9_950.0,
        });
        let decode = receiver_decode_context_from_stats(&stats, 10_000.0);
        let ctx = InsertContext::from_runtime(decode, &stats, 10_000.0, 50.0);
        let inspection = non_idr_inspection();
        assert!(ctx.supply_break_continuation);
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::Emit
        );
    }

    #[test]
    fn emit_decision_reports_deodable_reason() {
        let ctx = ctx_decoder_synced();
        let inspection = non_idr_inspection();
        let (decision, reason) = resolve_insert_decision_with_reason(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );
        assert_eq!(decision, InsertDecision::Emit);
        assert_eq!(reason, "decodableToFeed");
    }

    #[test]
    fn repairing_with_decoder_sync_emits_soft_missing_idr_delta() {
        let mut ctx = ctx_decoder_synced();
        ctx.decode.receiver_state = ReceiverState::Repairing;
        ctx.decode.has_active_gap = true;
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::Emit
        );
    }
}
