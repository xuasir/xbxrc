use super::source::{
    detect_forward_gap, resolve_inspection_admission, resolve_recovery_keyframe_action,
    InspectionAdmission, RecoveryKeyframeAction,
};
use super::NackSequenceWindow;
use crate::media::video::h264::inspection::{
    H264AccessUnitInspection, H264AccessUnitInspector, H264BootstrapRejectReason,
};
use crate::transport::webrtc::recovery::recovery_signal::VideoRecoverySignal;

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
        InspectionAdmission::Recover(VideoRecoverySignal::TransportAwaitRecoveryKeyframe)
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
