use super::receiver_state::ReceiverState;

/// 从接收源运行时事实推导 receiver-local 状态（不读 session/recovery owner）。
/// displayed IDR 正在 serving 时，等待 keyframe 的诊断态收敛到 receiving / repairing。
pub(crate) fn receiver_state_from_runtime(
    waiting_keyframe: bool,
    has_active_gap: bool,
    assembled_frame_count: u64,
    displayed_idr_serving: bool,
) -> ReceiverState {
    if waiting_keyframe {
        if displayed_idr_serving {
            return if has_active_gap && assembled_frame_count > 0 {
                ReceiverState::Repairing
            } else {
                ReceiverState::Receiving
            };
        }
        return if has_active_gap && assembled_frame_count > 0 {
            ReceiverState::Repairing
        } else {
            ReceiverState::WaitingKeyframe
        };
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

    #[test]
    fn maps_displayed_idr_serving_waiting_keyframe_to_receiving() {
        assert_eq!(
            receiver_state_from_runtime(true, false, 10, true),
            ReceiverState::Receiving
        );
    }
}
