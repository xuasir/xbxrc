//! Pre-decode 单点裁决：对齐 libwebrtc Insert 纪律（Emit / HoldRepair / DropCorrupt）。

use crate::media::video::h264::inspection::H264AccessUnitInspection;
use crate::transport::rtc::receive::{
    decode_gate::{
        inspection_bootstrap_blocks_delta_continuation, resolve_inspection_admission,
        should_block_non_keyframe_admission, steady_displayed_idr_delta_admits,
        DecodeCorruptionPolicy, InspectionAdmission, ReceiverDecodeContext,
    },
    ReceiverState,
};
use crate::transport::rtc::recovery::contract::{
    derive_media_supply_phase_from_stats, fresh_h264_idr_admission_from_stats,
    gap_keyframe_only_mode_active, is_soft_missing_idr_bootstrap_reject_reason,
    parameter_sets_change_strict_active_from_stats, recovery_supply_break_active_from_stats,
    recovery_timed_fallback_active_from_stats, resolve_gap_vs_keyframe_mode, GapVsKeyframeMode,
    MediaSupplyPhase,
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct InsertContext {
    pub decode: ReceiverDecodeContext,
    pub gap_mode: GapVsKeyframeMode,
    pub timed_fallback_active: bool,
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
            timed_fallback_active: recovery_timed_fallback_active_from_stats(stats, now_ms),
            fresh_idr_admission: fresh_h264_idr_admission_from_stats(stats, now_ms),
            post_parameter_sets_change_strict: parameter_sets_change_strict_active_from_stats(
                stats,
                now_ms,
                effective_rtt_ms,
            ),
            supply_break_continuation: recovery_supply_break_active_from_stats(stats, now_ms),
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
    if inspection.is_idr && media_dropped_packets > 0 {
        return InsertDecision::DropCorrupt;
    }
    if ctx.fresh_idr_admission && inspection.is_idr {
        return InsertDecision::Emit;
    }
    if ctx.timed_fallback_active && steady_displayed_idr_delta_admits(inspection, &ctx.decode) {
        return InsertDecision::Emit;
    }
    if ctx.post_parameter_sets_change_strict && !ctx.supply_break_continuation && !inspection.is_idr
    {
        return InsertDecision::HoldRepair;
    }
    if gap_keyframe_only_mode_active(ctx.gap_mode) && !inspection.is_idr {
        if ctx.supply_break_continuation
            && steady_displayed_idr_delta_admits(inspection, &ctx.decode)
        {
            return InsertDecision::Emit;
        }
        if matches!(ctx.media_supply_phase, MediaSupplyPhase::Priming)
            && steady_displayed_idr_delta_admits(inspection, &ctx.decode)
        {
            return InsertDecision::Emit;
        }
        return InsertDecision::HoldRepair;
    }
    if steady_displayed_idr_delta_admits(inspection, &ctx.decode) {
        return InsertDecision::Emit;
    }
    if inspection.bootstrap_ready {
        return InsertDecision::Emit;
    }
    if matches!(policy, DecodeCorruptionPolicy::StandardWebRtc)
        && matches!(ctx.decode.receiver_state, ReceiverState::Repairing)
        && inspection.delta_continuation_ready()
        && inspection.committed_sps_present()
        && inspection.committed_pps_present()
        && !ctx.decode.hard_gap_blocks_delta()
    {
        return InsertDecision::Emit;
    }
    if should_block_non_keyframe_admission(&ctx.decode) && !inspection.is_idr {
        return InsertDecision::HoldRepair;
    }
    match resolve_inspection_admission(inspection, &ctx.decode, policy) {
        InspectionAdmission::Accept => InsertDecision::Emit,
        InspectionAdmission::AwaitRecoveryKeyframe => {
            if ctx.decode.prior_output_established
                && !ctx.decode.hard_gap_blocks_delta()
                && inspection.delta_continuation_ready()
                && is_soft_missing_idr_bootstrap_reject_reason(
                    inspection
                        .bootstrap_reject_reason
                        .map(|reason| reason.as_str()),
                )
            {
                return InsertDecision::Emit;
            }
            if inspection_bootstrap_blocks_delta_continuation(inspection)
                && !ctx.decode.displayed_idr_serving
            {
                return InsertDecision::HoldRepair;
            }
            InsertDecision::HoldRepair
        }
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

    fn ctx_displayed_idr_serving() -> InsertContext {
        InsertContext {
            decode: ReceiverDecodeContext {
                receiver_state: ReceiverState::Repairing,
                has_active_gap: true,
                nack_exhausted: false,
                first_frame_acquired: true,
                prior_output_established: true,
                displayed_idr_serving: true,
            },
            gap_mode: GapVsKeyframeMode::RepairFirst,
            timed_fallback_active: false,
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
    fn displayed_idr_delta_emits_despite_bootstrap_missing_idr() {
        let ctx = ctx_displayed_idr_serving();
        let inspection = non_idr_inspection();
        assert!(!inspection.bootstrap_ready);
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::Emit
        );
    }

    #[test]
    fn insert_emit_bypass_matches_emit_decision_for_soft_missing_idr() {
        let ctx = ctx_displayed_idr_serving();
        let inspection = non_idr_inspection();
        assert!(insert_emit_permits_decode_without_bootstrap_ready(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
        ));
    }

    #[test]
    fn keyframe_only_mode_holds_non_idr() {
        let mut ctx = ctx_displayed_idr_serving();
        ctx.gap_mode = GapVsKeyframeMode::KeyframeOnly;
        ctx.decode.displayed_idr_serving = false;
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::HoldRepair
        );
    }

    #[test]
    fn keyframe_only_mode_holds_displayed_idr_soft_missing_idr_delta() {
        let mut ctx = ctx_displayed_idr_serving();
        ctx.gap_mode = GapVsKeyframeMode::KeyframeOnly;
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::HoldRepair
        );
        assert_eq!(
            recovery_keyframe_action_for_insert_decision(InsertDecision::HoldRepair),
            crate::transport::rtc::receive::decode_gate_eval::RecoveryKeyframeAction::WaitKeyframe
        );
    }

    #[test]
    fn post_parameter_sets_change_strict_holds_non_idr() {
        let mut ctx = ctx_displayed_idr_serving();
        ctx.post_parameter_sets_change_strict = true;
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::HoldRepair
        );
    }

    #[test]
    fn supply_break_stats_context_emits_repairing_delta() {
        use crate::transport::rtc::receive::decode_gate::receiver_decode_context_from_stats;
        use crate::XbxEngineMediaRuntimeStats;
        use crate::XbxEngineVideoReceiverObservation;

        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.recovery_playback_recovered_at_ms = Some(1.0);
        stats.recovery_displayed_idr_at_ms = Some(100.0);
        stats.submit_age_ms = Some(5_000.0);
        stats.latest_video_receiver_observation = Some(XbxEngineVideoReceiverObservation {
            observation_id: 1,
            receiver_state: "repairing".to_string(),
            gap_sequence: Some(1),
            gap_span: Some(1),
            nack_in_flight: true,
            keyframe_request_pending: false,
            bootstrap_reject_reason: None,
            observed_at_ms: 100.0,
        });
        let decode = receiver_decode_context_from_stats(&stats, 10_000.0);
        let ctx = InsertContext::from_runtime(decode, &stats, 10_000.0, 50.0);
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::Emit
        );
        assert_eq!(
            recovery_keyframe_action_for_insert_decision(InsertDecision::Emit),
            crate::transport::rtc::receive::decode_gate_eval::RecoveryKeyframeAction::Submit
        );
    }

    #[test]
    fn repairing_with_displayed_idr_emits_soft_missing_idr_delta() {
        let mut ctx = ctx_displayed_idr_serving();
        ctx.decode.receiver_state = ReceiverState::Repairing;
        ctx.decode.has_active_gap = true;
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::Emit
        );
        assert_eq!(
            recovery_keyframe_action_for_insert_decision(InsertDecision::Emit),
            crate::transport::rtc::receive::decode_gate_eval::RecoveryKeyframeAction::Submit
        );
    }
}
