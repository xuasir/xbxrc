use super::receiver_state::ReceiverState;

/// 从接收源运行时事实推导 receiver-local 状态（不读 session/recovery owner）。
pub(crate) fn receiver_state_from_runtime(
    waiting_keyframe: bool,
    has_active_gap: bool,
    assembled_frame_count: u64,
    collapse_waiting_keyframe_to_repairing: bool,
) -> ReceiverState {
    if waiting_keyframe {
        if collapse_waiting_keyframe_to_repairing && has_active_gap && assembled_frame_count > 0 {
            return ReceiverState::Repairing;
        }
        return ReceiverState::WaitingKeyframe;
    }
    if has_active_gap {
        return ReceiverState::Repairing;
    }
    if assembled_frame_count > 0 {
        ReceiverState::Receiving
    } else {
        ReceiverState::Priming
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_waiting_keyframe() {
        assert_eq!(
            receiver_state_from_runtime(true, false, 10, false),
            ReceiverState::WaitingKeyframe
        );
    }

    #[test]
    fn collapses_waiting_keyframe_to_repairing_when_displayed_idr_serving_with_gap() {
        assert_eq!(
            receiver_state_from_runtime(true, true, 10, true),
            ReceiverState::Repairing
        );
    }

    #[test]
    fn maps_repairing_gap() {
        assert_eq!(
            receiver_state_from_runtime(false, true, 10, false),
            ReceiverState::Repairing
        );
    }

    #[test]
    fn maps_priming_before_first_frame() {
        assert_eq!(
            receiver_state_from_runtime(false, false, 0, false),
            ReceiverState::Priming
        );
    }
}
