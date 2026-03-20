use std::time::Duration;

use crate::media::video::types::{AssembledVideoFrame, EncodedFrame, FrameValue};

// playout budget 属于 decode 前的准入语义，应放在 ingress 侧统一定义。
pub fn materialize_ingress_frame(
    frame: AssembledVideoFrame,
    min_delay: Duration,
    max_delay: Duration,
) -> EncodedFrame {
    let playout_delay = resolve_playout_delay(frame.value, min_delay, max_delay);
    let target_playout_time = frame.assembled_at + playout_delay;
    frame.into_encoded_frame(target_playout_time)
}

fn resolve_playout_delay(value: FrameValue, min_delay: Duration, max_delay: Duration) -> Duration {
    if value.is_sync_point() || value.refresh_boost {
        return max_delay.max(min_delay);
    }

    let ratio = value.deadline_budget_ratio_per_mille() as u128;
    let min_ms = min_delay.as_millis();
    let max_ms = max_delay.as_millis().max(min_ms);
    let spread_ms = max_ms.saturating_sub(min_ms);
    let scaled_ms = min_ms + (spread_ms * ratio / 1_000);
    Duration::from_millis(scaled_ms as u64).max(min_delay)
}

#[cfg(test)]
mod tests {
    use super::materialize_ingress_frame;
    use crate::media::video::h264::inspection::{
        H264AccessUnitInspection, H264BootstrapRejectReason,
    };
    use crate::media::video::types::{AssembledVideoFrame, FrameValue, VideoCodec};
    use bytes::Bytes;
    use std::time::{Duration, Instant};

    fn make_h264_inspection(bootstrap_ready: bool) -> H264AccessUnitInspection {
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
            slice_headers_valid: bootstrap_ready,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready,
            bootstrap_reject_reason: if bootstrap_ready {
                None
            } else {
                Some(H264BootstrapRejectReason::MissingSps)
            },
            commit_state:
                crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
        }
    }

    #[test]
    fn delta_frame_gets_tighter_playout_budget_than_keyframe() {
        let assembled_at = Instant::now();
        let keyframe = materialize_ingress_frame(
            AssembledVideoFrame {
                codec: VideoCodec::H264,
                is_keyframe: true,
                config_changed: true,
                value: FrameValue::new(true, true, 64 * 1024),
                width: 1920,
                height: 1080,
                rtp_timestamp: 1,
                assembled_at,
                h264: make_h264_inspection(true),
                payload: Bytes::from_static(b"k"),
            },
            Duration::from_millis(8),
            Duration::from_millis(30),
        );
        let delta = materialize_ingress_frame(
            AssembledVideoFrame {
                codec: VideoCodec::H264,
                is_keyframe: false,
                config_changed: false,
                value: FrameValue::new(false, false, 8 * 1024),
                width: 1920,
                height: 1080,
                rtp_timestamp: 2,
                assembled_at,
                h264: make_h264_inspection(false),
                payload: Bytes::from_static(b"d"),
            },
            Duration::from_millis(8),
            Duration::from_millis(30),
        );

        assert!(delta.target_playout_time < keyframe.target_playout_time);
    }
}
