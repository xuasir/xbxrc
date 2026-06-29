//! Pre-decode 单点裁决：对齐 libwebrtc Insert 纪律（Emit / HoldRepair / DropCorrupt）。

use crate::media::video::h264::inspection::H264AccessUnitInspection;
use crate::transport::rtc::receive::decode_gate::{
    insert_decodable_to_feed, inspection_bootstrap_blocks_delta_continuation,
    should_block_non_keyframe_admission, DecodeCorruptionPolicy, InspectionAdmission,
    ReceiverDecodeContext,
};
use crate::transport::rtc::recovery::contract::{
    decoder_waiting_keyframe_control_active_from_stats, fresh_idr_admission_from_control,
    normalize_action_stage_for_reference, parameter_sets_change_strict_from_control,
    resolve_gap_mode_from_control, supply_break_continuation_from_control, GapVsKeyframeMode,
    InsertControlTiming, PacketRecoveryActionStage, ReferenceChainObservation, ReferenceChainState,
};
use crate::transport::rtc::recovery::contract::{
    derive_packet_recovery_action_stage_from_stats, derive_reference_chain_state_from_stats,
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
    ActiveRepairHold,
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
            Self::ActiveRepairHold => "activeRepairHold",
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
    /// 与 `gap_keyframe_only_mode_active(gap_mode)` 联动。
    pub gap_mode: GapVsKeyframeMode,
    pub action_stage: PacketRecoveryActionStage,
    pub fresh_idr_admission: bool,
    /// PS/config 变更后短窗：非 IDR 一律 HoldRepair，优先 RTCP IDR。
    pub post_parameter_sets_change_strict: bool,
    /// supply-break 续播窄路径：仍允许 repairing delta Emit。
    pub supply_break_continuation: bool,
    pub reference_chain_state: ReferenceChainState,
    /// receive ledger `keyframe_required`：完整 usable keyframe 优先 Emit。
    pub keyframe_required: bool,
}

impl InsertContext {
    /// stats 投影路径：decode actor 等无 ledger 上下文时的 InsertContext 构造。
    pub(crate) fn from_runtime(
        decode: ReceiverDecodeContext,
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
        effective_rtt_ms: f64,
    ) -> Self {
        Self::from_runtime_with_reference(
            decode,
            stats,
            now_ms,
            effective_rtt_ms,
            derive_reference_chain_state_from_stats(stats, now_ms, effective_rtt_ms),
            stats.receive_keyframe_required.unwrap_or(false),
        )
    }

    pub(crate) fn from_runtime_with_reference(
        decode: ReceiverDecodeContext,
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
        effective_rtt_ms: f64,
        reference_chain: ReferenceChainObservation,
        keyframe_required: bool,
    ) -> Self {
        Self::from_ledger_inputs(
            decode,
            reference_chain,
            derive_packet_recovery_action_stage_from_stats(stats, now_ms, effective_rtt_ms),
            keyframe_required,
            stats,
            now_ms,
            effective_rtt_ms,
        )
    }

    /// Insert / feedback 单轨：主裁决只读 ledger 投影的 reference、action_stage、keyframe_required。
    pub(crate) fn from_ledger_inputs(
        decode: ReceiverDecodeContext,
        reference_chain: ReferenceChainObservation,
        action_stage: PacketRecoveryActionStage,
        keyframe_required: bool,
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
        effective_rtt_ms: f64,
    ) -> Self {
        let timing = InsertControlTiming {
            fresh_idr_inspection_accepted_at_ms: stats
                .latest_h264_inspection_observation
                .as_ref()
                .filter(|inspection| inspection.is_idr && inspection.admission_accepted)
                .map(|inspection| inspection.observed_at_ms),
            parameter_sets_changed_at_ms: stats.video_parameter_sets_changed_at_ms,
            gap_age_ms: stats
                .latest_video_timeline_observation
                .as_ref()
                .and_then(|timeline| timeline.gap.as_ref())
                .map(|gap| (now_ms - gap.observed_at_ms).max(0.0)),
            decoder_waiting_keyframe: decoder_waiting_keyframe_control_active_from_stats(
                stats, now_ms,
            ),
        };
        Self::from_ledger_control(
            decode,
            reference_chain,
            action_stage,
            keyframe_required,
            timing,
            now_ms,
            effective_rtt_ms,
        )
    }

    /// 控制面路径：不读 stats 派生 gap/supply 诊断投影参与 Hold/Emit。
    pub(crate) fn from_ledger_control(
        decode: ReceiverDecodeContext,
        reference_chain: ReferenceChainObservation,
        action_stage: PacketRecoveryActionStage,
        keyframe_required: bool,
        timing: InsertControlTiming,
        now_ms: f64,
        effective_rtt_ms: f64,
    ) -> Self {
        let action_stage = normalize_action_stage_for_reference(reference_chain, action_stage);
        let fresh_idr_admission = fresh_idr_admission_from_control(&timing, now_ms);
        let post_parameter_sets_change_strict = parameter_sets_change_strict_from_control(
            &timing,
            reference_chain,
            fresh_idr_admission,
            now_ms,
            effective_rtt_ms,
        );
        let supply_break_continuation =
            supply_break_continuation_from_control(reference_chain, action_stage);
        let gap_mode = resolve_gap_mode_from_control(
            reference_chain,
            action_stage,
            &timing,
            post_parameter_sets_change_strict,
            now_ms,
            effective_rtt_ms,
        );
        Self {
            decode,
            gap_mode,
            action_stage,
            fresh_idr_admission,
            post_parameter_sets_change_strict,
            supply_break_continuation,
            reference_chain_state: reference_chain.state,
            keyframe_required,
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
    if ctx.keyframe_required
        && inspection.is_idr
        && (inspection.bootstrap_ready || ctx.fresh_idr_admission)
    {
        return (InsertDecision::Emit, InsertDecisionReason::FreshIdr);
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
    if matches!(ctx.reference_chain_state, ReferenceChainState::NeedKeyframe) {
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
    if matches!(ctx.reference_chain_state, ReferenceChainState::Repairing)
        && matches!(
            ctx.action_stage,
            PacketRecoveryActionStage::NackPending | PacketRecoveryActionStage::NackMissed
        )
        && !inspection.is_idr
        && !inspection.bootstrap_ready
    {
        return (
            InsertDecision::HoldRepair,
            InsertDecisionReason::ActiveRepairHold,
        );
    }
    if insert_decodable_to_feed(
        inspection,
        &ctx.decode,
        ctx.action_stage,
        ctx.reference_chain_state,
    ) {
        return (InsertDecision::Emit, InsertDecisionReason::DecodableToFeed);
    }
    if insert_serviceable_continuation_to_feed(inspection, ctx) {
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

fn insert_serviceable_continuation_to_feed(
    inspection: &H264AccessUnitInspection,
    ctx: &InsertContext,
) -> bool {
    if inspection.is_idr || inspection.bootstrap_ready {
        return false;
    }
    if !inspection_bootstrap_blocks_delta_continuation(inspection) {
        return false;
    }
    if !ctx.decode.first_frame_acquired || ctx.keyframe_required {
        return false;
    }
    if ctx.post_parameter_sets_change_strict || ctx.decode.hard_gap_blocks_delta() {
        return false;
    }
    if matches!(ctx.reference_chain_state, ReferenceChainState::NeedKeyframe) {
        return false;
    }
    ctx.supply_break_continuation
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
    use crate::media::video::h264::inspection::{
        H264AccessUnitInspector, H264BootstrapRejectReason,
    };
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
                receiver_state: ReceiverState::Receiving,
                has_active_gap: false,
                nack_exhausted: false,
                first_frame_acquired: true,
                decoder_reference_synced: true,
            },
            gap_mode: GapVsKeyframeMode::RepairFirst,
            action_stage: PacketRecoveryActionStage::Steady,
            fresh_idr_admission: false,
            post_parameter_sets_change_strict: false,
            supply_break_continuation: false,
            reference_chain_state: ReferenceChainState::Continuous,
            keyframe_required: false,
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

    fn soft_missing_idr_continuation_inspection() -> H264AccessUnitInspection {
        H264AccessUnitInspection {
            nals: Vec::new(),
            parameter_sets: None,
            width: None,
            height: None,
            is_idr: false,
            has_inband_sps: false,
            has_inband_pps: false,
            slice_headers_valid: false,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some(H264BootstrapRejectReason::BootstrapMissingIdr),
            commit_state: H264AccessUnitInspector::test_commit_state(),
        }
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

    #[test]
    fn continuous_reference_masks_repair_action_stage_for_insert_gate() {
        let decode = ReceiverDecodeContext {
            receiver_state: ReceiverState::Repairing,
            has_active_gap: true,
            nack_exhausted: false,
            first_frame_acquired: true,
            decoder_reference_synced: true,
        };
        let reference = ReferenceChainObservation {
            state: ReferenceChainState::Continuous,
            cause: "ledger-decoder-reference-synced",
            decoder_reference_synced: true,
            has_active_gap: true,
            ..Default::default()
        };
        let ctx = InsertContext::from_ledger_control(
            decode,
            reference,
            PacketRecoveryActionStage::RequestIdr,
            false,
            InsertControlTiming {
                gap_age_ms: Some(6_000.0),
                ..Default::default()
            },
            10_000.0,
            50.0,
        );

        assert_eq!(ctx.action_stage, PacketRecoveryActionStage::Steady);
        assert_eq!(ctx.gap_mode, GapVsKeyframeMode::RepairFirst);
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

    fn ctx_first_frame_unsynced() -> InsertContext {
        InsertContext {
            decode: ReceiverDecodeContext {
                receiver_state: ReceiverState::WaitingKeyframe,
                has_active_gap: false,
                nack_exhausted: false,
                first_frame_acquired: false,
                decoder_reference_synced: false,
            },
            gap_mode: GapVsKeyframeMode::RepairFirst,
            action_stage: PacketRecoveryActionStage::WaitKeyframe,
            fresh_idr_admission: false,
            post_parameter_sets_change_strict: false,
            supply_break_continuation: false,
            reference_chain_state: ReferenceChainState::NeedKeyframe,
            keyframe_required: true,
        }
    }

    #[test]
    fn need_keyframe_holds_decodable_non_idr_before_first_frame() {
        let mut ctx = ctx_first_frame_unsynced();
        ctx.action_stage = PacketRecoveryActionStage::NackPending;
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
            reason == "decodableToFeed"
                || reason == "firstFrameFreshOrBootstrapIdr"
                || reason == "freshIdr",
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
        assert_eq!(hold_reason, "mustIdrHold");

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
        ctx.reference_chain_state = ReferenceChainState::NeedKeyframe;
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
        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::HoldRepair
        );
    }

    #[test]
    fn serviceable_anchor_supply_break_emits_soft_missing_idr_continuation() {
        let mut ctx = ctx_decoder_synced();
        ctx.action_stage = PacketRecoveryActionStage::Steady;
        ctx.supply_break_continuation = true;
        ctx.reference_chain_state = ReferenceChainState::Continuous;
        ctx.keyframe_required = false;
        let inspection = soft_missing_idr_continuation_inspection();

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
    fn need_keyframe_keeps_holding_soft_missing_idr_continuation() {
        let mut ctx = ctx_decoder_synced();
        ctx.action_stage = PacketRecoveryActionStage::Steady;
        ctx.supply_break_continuation = true;
        ctx.reference_chain_state = ReferenceChainState::NeedKeyframe;
        ctx.keyframe_required = true;
        let inspection = soft_missing_idr_continuation_inspection();

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
    fn decoder_reference_sync_collapses_stale_waiting_keyframe_without_active_gap() {
        use crate::transport::rtc::receive::decode_gate::receiver_decode_context_from_stats;
        use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoReceiverObservation};

        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(9_950.0);
        stats.recovery_playback_recovered_at_ms = Some(9_950.0);
        stats.recovery_decoder_reference_synced_at_ms = Some(9_950.0);
        stats.latest_video_decode_ok_time_ms = Some(9_950.0);
        stats.latest_video_receiver_observation = Some(XbxEngineVideoReceiverObservation {
            observation_id: 1,
            receiver_state: "waiting-keyframe".to_string(),
            gap_sequence: Some(1),
            gap_span: None,
            nack_in_flight: false,
            keyframe_request_pending: true,
            bootstrap_reject_reason: None,
            observed_at_ms: 10_000.0,
        });

        let decode = receiver_decode_context_from_stats(&stats, 10_000.0);
        assert_eq!(decode.receiver_state, ReceiverState::Receiving);
        assert!(!should_block_non_keyframe_admission(&decode));

        let ctx = InsertContext::from_runtime(decode, &stats, 10_000.0, 50.0);
        assert_eq!(ctx.decode.receiver_state, ReceiverState::Receiving);

        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::Emit
        );
    }

    #[test]
    fn clean_anchor_masks_stale_decoder_waiting_for_insert_control() {
        use crate::transport::rtc::receive::decode_gate::receiver_decode_context_from_stats;
        use crate::transport::rtc::recovery::contract::CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK;
        use crate::{
            XbxEngineDecodeOutputPathObservation, XbxEngineMediaRuntimeStats,
            XbxEngineVideoReceiverObservation,
        };

        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 3;
        stats.video_anchor_clean_epoch = Some(3);
        stats.video_anchor_clean_observed_at_ms = Some(9_950.0);
        stats.video_anchor_clean_source_event = Some("decoded-usable-idr".to_string());
        stats.recovery_fresh_anchor_recovered_at_ms = Some(9_950.0);
        stats.recovery_decoder_reference_synced_at_ms = Some(9_950.0);
        stats.latest_video_decode_ok_time_ms = Some(9_950.0);
        stats.latest_video_decode_ok_rtp_timestamp = Some(77_001);
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.latest_decode_output_path_observation = Some(XbxEngineDecodeOutputPathObservation {
            observation_id: 1,
            verdict: "backend-no-output".to_string(),
            detail: "backendNoOutputAfterWaitingKeyframeContinuation".to_string(),
            frame_rtp_timestamp: 77_001,
            is_keyframe: false,
            status: None,
            send_packet_status: None,
            receive_frame_status: None,
            backend_no_output_streak: Some(CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK),
            input_frames_since_last_decoded: Some(8),
            bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
            observed_at_ms: 9_940.0,
        });
        stats.latest_video_receiver_observation = Some(XbxEngineVideoReceiverObservation {
            observation_id: 1,
            receiver_state: "waiting-keyframe".to_string(),
            gap_sequence: None,
            gap_span: None,
            nack_in_flight: false,
            keyframe_request_pending: true,
            bootstrap_reject_reason: None,
            observed_at_ms: 9_940.0,
        });

        let decode = receiver_decode_context_from_stats(&stats, 10_000.0);
        assert_eq!(decode.receiver_state, ReceiverState::Receiving);
        let ctx = InsertContext::from_runtime(decode, &stats, 10_000.0, 50.0);
        assert_eq!(ctx.action_stage, PacketRecoveryActionStage::Steady);
        assert_eq!(ctx.gap_mode, GapVsKeyframeMode::RepairFirst);
        assert_eq!(ctx.reference_chain_state, ReferenceChainState::Continuous);

        let inspection = non_idr_inspection();
        assert_eq!(
            resolve_insert_decision(&inspection, &ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
            InsertDecision::Emit
        );
    }

    #[test]
    fn decoder_waiting_keyframe_reopens_window_and_holds_continuation() {
        use crate::transport::rtc::receive::decode_gate::receiver_decode_context_from_stats;
        use crate::transport::rtc::receive::recovery_ledger::ReceiveRecoveryLedger;
        use crate::XbxEngineMediaRuntimeStats;

        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_clean_anchor_committed(Some(77_001));
        ledger.note_decoder_reference_synced(9_950.0);
        ledger.note_decoder_waiting_keyframe();

        let mut stats = XbxEngineMediaRuntimeStats {
            recovery_playback_recovered_at_ms: Some(9_900.0),
            latest_video_decode_ok_time_ms: Some(9_950.0),
            latest_video_decode_ok_rtp_timestamp: Some(77_001),
            ..Default::default()
        };
        ledger.sync_to_stats(&mut stats);

        let decode = receiver_decode_context_from_stats(&stats, 10_000.0);
        let reference = ReferenceChainObservation {
            state: ReferenceChainState::NeedKeyframe,
            cause: "decoder-waiting-keyframe",
            ..Default::default()
        };
        let ctx = InsertContext::from_ledger_inputs(
            decode,
            reference,
            ledger.derive_packet_recovery_action_stage(false, false, None, 50.0),
            ledger.keyframe_required,
            &stats,
            10_000.0,
            50.0,
        );
        let inspection = non_idr_inspection();
        let (decision, reason) = resolve_insert_decision_with_reason(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );

        assert_eq!(reference.state, ReferenceChainState::NeedKeyframe);
        assert_eq!(decision, InsertDecision::HoldRepair);
        assert_eq!(reason, "mustIdrHold");
    }

    #[test]
    fn nack_exhausted_reopens_window_and_holds_continuation_after_clean_anchor() {
        use crate::transport::rtc::receive::decode_gate::receiver_decode_context_from_stats;
        use crate::transport::rtc::receive::recovery_ledger::ReceiveRecoveryLedger;
        use crate::XbxEngineMediaRuntimeStats;

        let mut ledger = ReceiveRecoveryLedger::default();
        ledger.note_clean_anchor_committed(Some(77_001));
        ledger.note_decoder_reference_synced(9_950.0);
        ledger.note_nack_exhausted();

        let mut stats = XbxEngineMediaRuntimeStats {
            recovery_playback_recovered_at_ms: Some(9_900.0),
            latest_video_decode_ok_time_ms: Some(9_950.0),
            latest_video_decode_ok_rtp_timestamp: Some(77_001),
            ..Default::default()
        };
        ledger.sync_to_stats(&mut stats);

        let decode = receiver_decode_context_from_stats(&stats, 10_000.0);
        let reference = ledger.project_reference_chain(false, true, &Default::default());
        let ctx = InsertContext::from_ledger_inputs(
            decode,
            reference,
            ledger.derive_packet_recovery_action_stage(true, false, Some(120.0), 50.0),
            ledger.keyframe_required,
            &stats,
            10_000.0,
            50.0,
        );
        let inspection = non_idr_inspection();
        let (decision, reason) = resolve_insert_decision_with_reason(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );

        assert_eq!(reference.state, ReferenceChainState::NeedKeyframe);
        assert_eq!(reference.cause, "nack-exhausted");
        assert_eq!(decision, InsertDecision::HoldRepair);
        assert_eq!(reason, "mustIdrHold");
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
    fn post_parameter_sets_strict_uses_packet_reference_decoder_evidence_without_display_stable() {
        let decode = ReceiverDecodeContext {
            receiver_state: ReceiverState::Repairing,
            has_active_gap: true,
            nack_exhausted: false,
            first_frame_acquired: true,
            decoder_reference_synced: false,
        };
        let reference = ReferenceChainObservation {
            state: ReferenceChainState::Unknown,
            cause: "bootstrap-missing-priming",
            decoder_reference_synced: false,
            has_active_gap: false,
            bootstrap_ready: false,
            ..Default::default()
        };
        let ctx = InsertContext::from_ledger_control(
            decode,
            reference,
            PacketRecoveryActionStage::Steady,
            false,
            InsertControlTiming {
                parameter_sets_changed_at_ms: Some(9_980.0),
                ..Default::default()
            },
            10_000.0,
            40.0,
        );
        let inspection = non_idr_inspection();
        let (decision, reason) = resolve_insert_decision_with_reason(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );

        assert!(ctx.post_parameter_sets_change_strict);
        assert_eq!(decision, InsertDecision::HoldRepair);
        assert_eq!(reason, "postPsStrict");
    }

    #[test]
    fn submit_starved_stats_context_emits_non_idr_when_decoder_synced() {
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
            InsertDecision::Emit
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
    fn repairing_with_decoder_sync_holds_soft_missing_idr_delta_during_active_repair() {
        let mut ctx = ctx_decoder_synced();
        ctx.decode.receiver_state = ReceiverState::Repairing;
        ctx.decode.has_active_gap = true;
        ctx.reference_chain_state = ReferenceChainState::Repairing;
        ctx.action_stage = PacketRecoveryActionStage::NackPending;
        let inspection = non_idr_inspection();
        let (decision, reason) = resolve_insert_decision_with_reason(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );
        assert_eq!(decision, InsertDecision::HoldRepair);
        assert_eq!(reason, "activeRepairHold");
    }

    #[test]
    fn continuous_synced_active_gap_keeps_steady_and_emits_continuation() {
        let decode = ReceiverDecodeContext {
            receiver_state: ReceiverState::Repairing,
            has_active_gap: true,
            nack_exhausted: false,
            first_frame_acquired: true,
            decoder_reference_synced: true,
        };
        let reference = ReferenceChainObservation {
            state: ReferenceChainState::Continuous,
            cause: "reference-continuous",
            decoder_reference_synced: true,
            has_active_gap: true,
            ..Default::default()
        };
        let ctx = InsertContext::from_ledger_control(
            decode,
            reference,
            PacketRecoveryActionStage::Steady,
            false,
            InsertControlTiming {
                gap_age_ms: Some(120.0),
                ..Default::default()
            },
            10_000.0,
            40.0,
        );
        let inspection = non_idr_inspection();
        let (decision, reason) = resolve_insert_decision_with_reason(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );

        assert_eq!(ctx.action_stage, PacketRecoveryActionStage::Steady);
        assert_eq!(decision, InsertDecision::Emit);
        assert_eq!(reason, "decodableToFeed");
    }

    #[test]
    fn repairing_synced_active_gap_normalizes_steady_stage_to_active_repair_hold() {
        let decode = ReceiverDecodeContext {
            receiver_state: ReceiverState::Repairing,
            has_active_gap: true,
            nack_exhausted: false,
            first_frame_acquired: true,
            decoder_reference_synced: true,
        };
        let reference = ReferenceChainObservation {
            state: ReferenceChainState::Repairing,
            cause: "receive-ledger-hard-gap-repairing",
            decoder_reference_synced: true,
            has_active_gap: true,
            ..Default::default()
        };
        let ctx = InsertContext::from_ledger_control(
            decode,
            reference,
            PacketRecoveryActionStage::Steady,
            false,
            InsertControlTiming {
                gap_age_ms: Some(120.0),
                ..Default::default()
            },
            10_000.0,
            40.0,
        );
        let inspection = non_idr_inspection();
        let (decision, reason) = resolve_insert_decision_with_reason(
            &inspection,
            &ctx,
            DecodeCorruptionPolicy::StandardWebRtc,
            0,
        );

        assert_eq!(ctx.action_stage, PacketRecoveryActionStage::NackMissed);
        assert_eq!(decision, InsertDecision::HoldRepair);
        assert_eq!(reason, "activeRepairHold");
    }
}
