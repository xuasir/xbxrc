//! 动态 RTT 恢复时序合同测试：NACK 首发窗与 survival、profile 静态回退、FIR 仅 Cloud、H264 参数集缓存 bootstrap。
//! 对应设计说明见 `docs/rfcs/2026-05-14-dynamic-rtt-aware-recovery-timing.md`。

use std::sync::{Arc, Mutex};

use hex_literal::hex;
use xbxengine_protocol::XbxEngineTargetTypeDto;

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::media::video::h264::inspection::H264AccessUnitInspector;
use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::FrameValue;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState;
use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
use crate::transport::rtc::recovery::policy::{
    RecoveryTimingRttParams, ScenarioPolicyProfileKind, ScenarioPolicyResolver,
};
use crate::transport::rtc::recovery::timing::{
    merge_nack_admission_deadline_with_dynamic_timeout, resolve_recovery_dynamic_timing_with_rtt,
};
use crate::transport::rtc::session::policy::RecoveryAction;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::nack_scheduler::{
    NackObservePolicy, NackScheduler, NackSchedulerConfig, PacketRecoveryDisposition,
};

#[test]
fn home_wan_supply_gap_does_not_escalate_before_dynamic_first_attempt_timeout() {
    let merged = merge_nack_admission_deadline_with_dynamic_timeout(
        1_000.0,
        1_020.0,
        "supply",
        120.0,
        Some(2_500.0),
    );
    assert!(
        merged >= 1_120.0,
        "高价值缺口 admission deadline 至少覆盖一轮动态 NACK 超时"
    );
}

#[test]
fn continuation_only_waits_dynamic_patience_window_before_pli_refresh() {
    let profile = ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
        ScenarioPolicyProfileKind::RelayGaming,
    );
    let t100 = resolve_recovery_dynamic_timing_with_rtt(100.0, profile);
    let t200 = resolve_recovery_dynamic_timing_with_rtt(200.0, profile);
    assert!(t200.continuation_patience_window_ms > t100.continuation_patience_window_ms);
}

#[test]
fn timing_rtt_shape_static_pli_when_dim_absent() {
    let mut profile = ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
        ScenarioPolicyProfileKind::HomeLanGaming,
    );
    profile.timing_rtt = Some(RecoveryTimingRttParams::default());
    let t = resolve_recovery_dynamic_timing_with_rtt(10.0, profile);
    assert!(
        (t.pli_refresh_interval_ms - profile.pli_refresh_interval_ms).abs() < 0.01,
        "timing_rtt 存在但 pli_refresh 维度未配置时回退 profile 静态 PLI 间隔"
    );
}

#[test]
fn recovery_profile_enables_dynamic_pli_and_fir_timing_dimensions() {
    let profile = ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
        ScenarioPolicyProfileKind::CloudGaming,
    );
    let t100 = resolve_recovery_dynamic_timing_with_rtt(100.0, profile);
    let t200 = resolve_recovery_dynamic_timing_with_rtt(200.0, profile);
    assert!(t200.pli_refresh_interval_ms > t100.pli_refresh_interval_ms);
    assert!(t200.fir_retry_interval_ms > t100.fir_retry_interval_ms);
}

#[test]
fn cloud_high_rtt_reference_gap_prefers_wider_first_attempt_window_before_retry_or_pli() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 2_000,
        frame_deadline_ms: 50,
        burst_count: 1,
        retry_interval_ms: 5,
        max_retry_count: 1,
    });
    let p = NackObservePolicy {
        source: "sampleLoss",
        deadline_at_ms: Some(3_000.0),
        max_age_ms: Some(1_800),
        retry_interval_ms: Some(5),
        burst_count: Some(1),
        max_tracked_sequences: Some(4),
        frame_rtp_timestamp: Some(90_000),
        frame_is_keyframe: Some(false),
        frame_importance: "supply",
        priority: 2,
        budget_context: FrameBudgetContext::steady_for_value(FrameValue::new(
            false,
            true,
            48 * 1024,
        )),
        estimated_recovery_arrival_ms: Some(150.0),
        frame_playout_deadline_at_ms: Some(3_000.0),
        nack_disposition: PacketRecoveryDisposition::Attempted,
        frame_unrecoverable_reason: None,
        max_retry_count_override: Some(1),
        first_attempt_survival_window_ms: Some(200.0),
        repairability_schedule: Some(0.9),
        admission_deadline_floor_at_ms: None,
    };
    let (b, _) = scheduler.observe_missing_sequences_with_policy(&[7], 100.0, p);
    assert!(b.is_some());
    let polled = scheduler.poll(105.0);
    assert!(polled.retry_batch.is_none());
    assert_eq!(
        polled
            .recovery_telemetry
            .as_ref()
            .and_then(|t| t.retry_suppressed_reason.as_deref()),
        Some("firstAttemptSurvival")
    );
    let polled2 = scheduler.poll(310.0);
    assert!(polled2.retry_batch.is_some());
    assert_eq!(
        polled2
            .recovery_telemetry
            .as_ref()
            .and_then(|t| t.retry_allowed_reason.as_deref()),
        Some("firstAttemptWindowElapsed")
    );
}

#[test]
fn fir_is_cloud_only_and_requires_failed_pli_progress() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let owner_signal = crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "transportAwaitRecoveryAnchor".to_string(),
        observed_at_ms: 12_180.0,
        gap_severity: None,
        repairability: None,
    };
    let proposal = crate::transport::rtc::recovery::coordinator::CoordinatorProposal {
        decision: crate::transport::rtc::recovery::escalation::VideoEscalationDecision {
            observation_id: 1,
            action: RecoveryAction::CoalescedKeyframeInFlight,
        },
        coalescing_mode: None,
        unlock_reason: None,
        preempt_reason: None,
        budget_before: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 45,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 0,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 1,
        },
        budget_after: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 45,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 0,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 1,
        },
    };
    RuntimeStatsSink::update_shared(runtime_stats.as_ref(), |stats| {
        stats.session_target_type = Some(XbxEngineTargetTypeDto::Home);
        stats.video_rtt_ms = Some(50.0);
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 45;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 45,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "packet-seen".to_string(),
                status_detail: None,
                requested_at_ms: 12_000.0,
                sent_at_ms: Some(12_010.0),
                deadline_at_ms: Some(12_900.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(12_070.0),
                first_video_packet_rtp_timestamp: Some(0x5566_7788),
                first_video_packet_is_keyframe: Some(true),
                first_keyframe_packet_at_ms: Some(12_070.0),
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(0x5566_7788),
                response_frame_seq: Some(45),
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: Some("packetSeen".to_string()),
                retired_at_ms: None,
            });
    });
    assert!(
        policy
            .should_upgrade_transport_await_refresh_to_fir(
                VideoSchedulingOwnerState::RebuildingSupply,
                &proposal,
                &owner_signal,
                12_180.0,
            )
            .is_none(),
        "非 Cloud 会话不得升级到 FIR"
    );
}

#[test]
fn bootstrap_missing_sps_uses_cached_parameter_sets_when_config_unchanged() {
    let inspector = H264AccessUnitInspector::new();
    let bootstrap_payload = hex!(
        "00 00 00 01 67 64 00 0A AC 72 84 44 26 84 00 00
         03 00 04 00 00 03 00 CA 3C 48 96 11 80 00 00 00
         01 68 E8 43 8F 13 21 30 00 00 01 65 88 81 00 05
         4E 7F 87 DF"
    );
    let bootstrap = inspector
        .inspect_access_unit(&bootstrap_payload)
        .expect("bootstrap inspection");
    bootstrap.commit();

    let idr_without_sets = hex!("00 00 00 01 65 88 81 00 05 4E 7F 87 DF");
    let inspection = inspector
        .inspect_access_unit(&idr_without_sets)
        .expect("idr inspection");

    assert!(inspection.bootstrap_ready);
    assert_eq!(inspection.bootstrap_reject_reason, None);
    assert!(inspection.parameter_sets.is_some());
    assert!(!inspection.has_inband_sps);
    assert!(!inspection.has_inband_pps);
}
