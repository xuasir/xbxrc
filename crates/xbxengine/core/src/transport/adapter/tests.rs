use super::nack::{
    cloud_startup_head_hole_deadline_at_ms, frame_value_for_importance, rtp_gap_nack_policy,
    rtp_window_nack_policy, sample_loss_nack_policy,
};
use super::source::{
    detect_forward_gap, resolve_inspection_admission, resolve_recovery_keyframe_action,
    InspectionAdmission, RecoveryKeyframeAction,
};
use super::NackSequenceWindow;
use crate::media::video::h264::inspection::{
    H264AccessUnitInspection, H264AccessUnitInspector, H264BootstrapRejectReason,
};

#[test]
fn nack_sequence_window_tracks_missing_and_wrap() {
    let mut window = NackSequenceWindow::new(1);
    window.add(10);
    window.add(11);
    window.add(13);
    assert_eq!(window.missing_seq_numbers(0), vec![12]);
    assert_eq!(window.missing_seq_numbers_in_range(10, 14), vec![12]);

    let mut wrapped = NackSequenceWindow::new(1);
    wrapped.add(u16::MAX);
    wrapped.add(0);
    wrapped.add(2);
    assert_eq!(wrapped.missing_seq_numbers(0), vec![1]);
}

#[test]
fn recovery_keyframe_action_only_waits_after_repeated_sample_loss() {
    assert_eq!(
        resolve_recovery_keyframe_action(false, 1, 3, false),
        (false, RecoveryKeyframeAction::DropAndRequestKeyframe)
    );
    assert_eq!(
        resolve_recovery_keyframe_action(false, 2, 3, false),
        (true, RecoveryKeyframeAction::TriggerWaitKeyframe)
    );
    assert_eq!(
        resolve_recovery_keyframe_action(true, 0, 0, false),
        (true, RecoveryKeyframeAction::WaitKeyframe)
    );
    assert_eq!(
        resolve_recovery_keyframe_action(true, 0, 0, true),
        (false, RecoveryKeyframeAction::Submit)
    );
    assert_eq!(
        resolve_recovery_keyframe_action(false, 0, 1, true),
        (true, RecoveryKeyframeAction::TriggerWaitKeyframe)
    );
    assert_eq!(
        resolve_recovery_keyframe_action(true, 1, 2, true),
        (true, RecoveryKeyframeAction::TriggerWaitKeyframe)
    );
}

#[test]
fn detect_forward_gap_ignores_old_out_of_order_packets() {
    assert_eq!(detect_forward_gap(None, 10), (Some(10), None));
    assert_eq!(detect_forward_gap(Some(10), 11), (Some(11), None));
    assert_eq!(detect_forward_gap(Some(10), 13), (Some(13), Some((11, 13))));
    assert_eq!(detect_forward_gap(Some(13), 12), (Some(13), None));
}

fn make_h264_inspection(
    bootstrap_ready: bool,
    slice_headers_valid: bool,
) -> H264AccessUnitInspection {
    H264AccessUnitInspection {
        nals: Vec::new(),
        parameter_sets: None,
        width: Some(1920),
        height: Some(1080),
        is_idr: bootstrap_ready,
        has_vcl: true,
        has_inband_sps: bootstrap_ready,
        has_inband_pps: bootstrap_ready,
        has_aud: false,
        slice_headers_valid,
        parameter_sets_changed: false,
        config_changed: false,
        bootstrap_ready,
        bootstrap_reject_reason: if slice_headers_valid {
            None
        } else {
            Some(H264BootstrapRejectReason::InvalidSliceHeader)
        },
        commit_state: H264AccessUnitInspector::test_commit_state(),
    }
}

#[test]
fn invalid_slice_header_au_requests_wait_keyframe_recovery() {
    let inspection = make_h264_inspection(false, false);

    assert_eq!(
        resolve_inspection_admission(&inspection),
        InspectionAdmission::AwaitRecoveryKeyframe
    );
}

#[test]
fn valid_slice_header_au_is_admitted_by_adapter_gate() {
    let inspection = make_h264_inspection(true, true);

    assert_eq!(
        resolve_inspection_admission(&inspection),
        InspectionAdmission::Accept
    );
}

#[test]
fn sample_loss_policy_scales_with_repairability() {
    let low = sample_loss_nack_policy(90_000, false, "reference", 1_200.0, 0.3, false, false);
    let high = sample_loss_nack_policy(90_000, false, "reference", 1_200.0, 0.95, false, false);

    assert_eq!(low.source, "sampleLoss");
    assert_eq!(high.source, "sampleLoss");
    assert!(high.max_age_ms.unwrap_or_default() > low.max_age_ms.unwrap_or_default());
    assert!(high.retry_interval_ms.unwrap_or_default() < low.retry_interval_ms.unwrap_or_default());
    assert!(high.burst_count.unwrap_or_default() >= low.burst_count.unwrap_or_default());
    assert!(high.priority >= low.priority);
}

#[test]
fn cloud_startup_head_hole_deadline_floor_only_applies_in_startup() {
    let now_ms = 1_000.0;
    let tight_deadline = 1_120.0;
    let wide_deadline = 1_450.0;

    assert_eq!(
        cloud_startup_head_hole_deadline_at_ms(now_ms, tight_deadline, false),
        tight_deadline
    );
    assert_eq!(
        cloud_startup_head_hole_deadline_at_ms(now_ms, tight_deadline, true),
        1_320.0
    );
    assert_eq!(
        cloud_startup_head_hole_deadline_at_ms(now_ms, wide_deadline, true),
        wide_deadline
    );
}

#[test]
fn cloud_startup_sample_loss_policy_is_wider_than_cloud_steady_and_home() {
    let home = sample_loss_nack_policy(90_000, false, "delta", 1_200.0, 0.7, false, false);
    let cloud_steady = sample_loss_nack_policy(90_000, false, "delta", 1_200.0, 0.7, true, false);
    let cloud_startup = sample_loss_nack_policy(90_000, false, "delta", 1_200.0, 0.7, true, true);

    assert!(
        cloud_startup.max_age_ms.unwrap_or_default() > cloud_steady.max_age_ms.unwrap_or_default()
    );
    assert!(cloud_steady.max_age_ms.unwrap_or_default() > home.max_age_ms.unwrap_or_default());
    assert!(
        cloud_startup.retry_interval_ms.unwrap_or_default()
            > cloud_steady.retry_interval_ms.unwrap_or_default()
    );
    assert!(
        cloud_steady.retry_interval_ms.unwrap_or_default()
            > home.retry_interval_ms.unwrap_or_default()
    );
    assert!(
        cloud_startup.max_tracked_sequences.unwrap_or_default()
            > cloud_steady.max_tracked_sequences.unwrap_or_default()
    );
    assert!(
        cloud_steady.max_tracked_sequences.unwrap_or_default()
            > home.max_tracked_sequences.unwrap_or_default()
    );
}

#[test]
fn cloud_startup_rtp_gap_and_window_policies_widen_head_hole_budget() {
    let value = frame_value_for_importance("reference");
    let home_gap = rtp_gap_nack_policy(value, 1_200.0, false, false);
    let cloud_steady_gap = rtp_gap_nack_policy(value, 1_200.0, true, false);
    let cloud_startup_gap = rtp_gap_nack_policy(value, 1_200.0, true, true);
    let home_window = rtp_window_nack_policy(value, 1_200.0, false, false);
    let cloud_steady_window = rtp_window_nack_policy(value, 1_200.0, true, false);
    let cloud_startup_window = rtp_window_nack_policy(value, 1_200.0, true, true);

    assert!(
        cloud_startup_gap.max_age_ms.unwrap_or_default()
            > cloud_steady_gap.max_age_ms.unwrap_or_default()
    );
    assert!(
        cloud_steady_gap.max_age_ms.unwrap_or_default() > home_gap.max_age_ms.unwrap_or_default()
    );
    assert!(
        cloud_startup_gap.max_tracked_sequences.unwrap_or_default()
            > cloud_steady_gap.max_tracked_sequences.unwrap_or_default()
    );
    assert!(
        cloud_steady_gap.max_tracked_sequences.unwrap_or_default()
            > home_gap.max_tracked_sequences.unwrap_or_default()
    );
    assert!(
        cloud_startup_window.max_age_ms.unwrap_or_default()
            > cloud_steady_window.max_age_ms.unwrap_or_default()
    );
    assert!(
        cloud_steady_window.max_age_ms.unwrap_or_default()
            > home_window.max_age_ms.unwrap_or_default()
    );
    assert!(
        cloud_startup_window
            .max_tracked_sequences
            .unwrap_or_default()
            > cloud_steady_window
                .max_tracked_sequences
                .unwrap_or_default()
    );
    assert!(
        cloud_steady_window
            .max_tracked_sequences
            .unwrap_or_default()
            > home_window.max_tracked_sequences.unwrap_or_default()
    );
}

#[test]
fn frame_value_for_importance_maps_sync_and_refresh_flags() {
    let keyframe = frame_value_for_importance("keyframe");
    let reference = frame_value_for_importance("reference");
    let delta = frame_value_for_importance("delta");

    assert!(keyframe.is_sync_point());
    assert!(!reference.is_sync_point());
    assert!(reference.refresh_boost);
    assert!(!delta.refresh_boost);
}
