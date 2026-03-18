use super::source::{detect_forward_gap, resolve_recovery_keyframe_action, RecoveryKeyframeAction};
use super::NackSequenceWindow;

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
