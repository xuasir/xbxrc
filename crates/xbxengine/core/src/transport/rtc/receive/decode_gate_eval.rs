//! pre-decode AU 裁决（RFC：`DecodeGate` / `RtcReceiveCore`）。

use super::UINT16SIZE_HALF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryKeyframeAction {
    Submit,
    DropAndRequestPli,
    WaitKeyframe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FirstFrameAcquisitionRequestKind {
    Initial,
    Followup,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FirstFrameAcquisitionRuntimeContext {
    pub(crate) session_is_startup: bool,
    pub(crate) transport_connected: bool,
    pub(crate) answer_missing_sprop: bool,
    pub(crate) first_frame_acquired: bool,
    pub(crate) audio_started: bool,
    pub(crate) video_track_audio_only: bool,
    pub(crate) video_track_media_seen: bool,
}

impl FirstFrameAcquisitionRuntimeContext {
    pub(crate) fn chain_active(&self) -> bool {
        self.session_is_startup
            && self.transport_connected
            && self.answer_missing_sprop
            && !self.first_frame_acquired
    }

    pub(crate) fn followup_evidence_ready(&self) -> bool {
        self.audio_started || self.video_track_audio_only || self.video_track_media_seen
    }
}

pub(crate) fn resolve_recovery_keyframe_action(
    first_frame_acquired: bool,
    is_blocking_non_keyframe_admission: bool,
    sustaining_recovery_active: bool,
    receiver_repairing: bool,
    hard_recovery_gap_risk: bool,
    _sample_loss_burst_count: u8,
    media_dropped_packets: u16,
    is_keyframe: bool,
    displayed_idr_serving: bool,
) -> (bool, RecoveryKeyframeAction) {
    // 带丢包的 keyframe/reference 不能继续喂给解码器，否则很容易把本地参考链喂脏，
    // 在 macOS 上会直接放大成 VideoToolbox 连续 bad-data 回调。
    if is_keyframe && media_dropped_packets > 0 {
        // 这里仍只保留 decoder safety：丢弃坏 keyframe，但恢复升级交给统一 NACK/recovery admission。
        return (false, RecoveryKeyframeAction::DropAndRequestPli);
    }

    if is_keyframe {
        return (false, RecoveryKeyframeAction::Submit);
    }

    if !first_frame_acquired {
        return (true, RecoveryKeyframeAction::WaitKeyframe);
    }

    if media_dropped_packets > 0 {
        // sample loss 的升级门交给统一 NACK/recovery admission；source 这里只保留解码安全职责。
        return (false, RecoveryKeyframeAction::DropAndRequestPli);
    }

    if is_blocking_non_keyframe_admission {
        // `displayed_idr_serving` 在 ingress 侧已收紧为 allows_relaxed_controls（供给断裂时不强制 Submit）。
        if displayed_idr_serving && first_frame_acquired {
            return (false, RecoveryKeyframeAction::Submit);
        }
        if !first_frame_acquired {
            return (true, RecoveryKeyframeAction::WaitKeyframe);
        }
        if sustaining_recovery_active || receiver_repairing {
            return (false, RecoveryKeyframeAction::Submit);
        }
        if !hard_recovery_gap_risk {
            return (false, RecoveryKeyframeAction::Submit);
        }
        return (true, RecoveryKeyframeAction::WaitKeyframe);
    }

    (false, RecoveryKeyframeAction::Submit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_idr_serving_avoids_wait_keyframe_while_blocking() {
        let (blocking, action) =
            resolve_recovery_keyframe_action(true, true, false, false, true, 0, 0, false, true);
        assert!(!blocking);
        assert_eq!(action, RecoveryKeyframeAction::Submit);
    }

    #[test]
    fn displayed_idr_serving_off_while_blocking_waits_for_keyframe_on_hard_gap() {
        let (blocking, action) =
            resolve_recovery_keyframe_action(true, true, false, false, true, 0, 0, false, false);
        assert!(blocking);
        assert_eq!(action, RecoveryKeyframeAction::WaitKeyframe);
    }
}

pub(crate) fn detect_forward_gap(
    last_highest_rtp_sequence: Option<u16>,
    sequence: u16,
) -> (Option<u16>, Option<(u16, u16)>) {
    let Some(last_highest) = last_highest_rtp_sequence else {
        return (Some(sequence), None);
    };
    let diff = sequence.wrapping_sub(last_highest);
    if diff == 0 {
        return (Some(last_highest), None);
    }
    if diff < UINT16SIZE_HALF {
        if diff > 1 {
            return (
                Some(sequence),
                Some((last_highest.wrapping_add(1), sequence)),
            );
        }
        return (Some(sequence), None);
    }

    (Some(last_highest), None)
}
