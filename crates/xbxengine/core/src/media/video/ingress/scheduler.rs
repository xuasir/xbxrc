use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::media::video::types::{EncodedFrame, VideoCodec};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngressDecision {
    Submit,
    DropLate,
    DropBacklog,
    WaitKeyframe,
    Reconfigure,
}

pub trait FrameScheduler: Send {
    fn submit(&mut self, frame: EncodedFrame, now: Instant) -> IngressDecision;
    fn pop(&mut self) -> Option<EncodedFrame>;
}

/// VideoIngress 负责根据 RFC 的规则过滤/调度网络侧输入的视频帧，防止阻塞解码流水线。
pub struct VideoIngress {
    queue: VecDeque<EncodedFrame>,
    max_size: usize,
    late_frame_drop_threshold: Duration,
    waiting_keyframe: bool,
    current_width: u32,
    current_height: u32,
    current_codec: Option<VideoCodec>,
}

impl VideoIngress {
    pub fn new(max_size: usize, late_frame_drop_threshold: Duration) -> Self {
        Self {
            queue: VecDeque::with_capacity(max_size),
            max_size,
            late_frame_drop_threshold,
            waiting_keyframe: true,
            current_width: 0,
            current_height: 0,
            current_codec: None,
        }
    }

    /// 主动清除队列，进入重配状态直到收到 Keyframe
    pub fn start_reconfigure(&mut self) {
        self.queue.clear();
        self.waiting_keyframe = true;
    }

    pub fn queue_depth(&self) -> usize {
        self.queue.len()
    }
}

impl FrameScheduler for VideoIngress {
    fn submit(&mut self, frame: EncodedFrame, now: Instant) -> IngressDecision {
        if frame.config_changed {
            if self.waiting_keyframe {
                return IngressDecision::WaitKeyframe;
            }
            self.start_reconfigure();
            return IngressDecision::Reconfigure;
        }

        // 基础参数变化也会触发必须使用 Keyframe 初始化
        let config_mismatch = self.current_codec.as_ref() != Some(&frame.codec)
            || self.current_width != frame.width
            || self.current_height != frame.height;

        if config_mismatch && !frame.is_keyframe {
            if self.waiting_keyframe {
                return IngressDecision::WaitKeyframe;
            }
            self.start_reconfigure();
            return IngressDecision::Reconfigure;
        }

        // Rule 1: Keyframe
        if frame.is_keyframe {
            self.current_codec = Some(frame.codec.clone());
            self.current_width = frame.width;
            self.current_height = frame.height;
            self.waiting_keyframe = false;

            // 永远优先: 清空 backlog
            self.queue.clear();
            self.queue.push_back(frame);
            return IngressDecision::Submit;
        }

        // 丢弃期间等待关键帧
        if self.waiting_keyframe {
            return IngressDecision::WaitKeyframe;
        }

        let frame_late_threshold = scale_duration_by_per_mille(
            self.late_frame_drop_threshold,
            frame.value.late_budget_ratio_per_mille(),
            Duration::from_millis(33),
        );

        if now > frame.target_playout_time + frame_late_threshold {
            crate::xbx_log_warn!(
                "[VideoIngress] frame too late, dropping. now={:?}, target={:?}",
                now,
                frame.target_playout_time
            );
            return IngressDecision::DropLate;
        }

        // Rule 3: Backlog 控制
        if self.queue.len() >= self.max_size {
            // backlog 时基于帧价值模型做替换，避免低价值旧帧继续堵塞队列。
            if let Some((lowest_idx, lowest_score)) = self
                .queue
                .iter()
                .enumerate()
                .map(|(idx, queued)| (idx, queued.value.backlog_priority_score()))
                .min_by_key(|(_, score)| *score)
            {
                let incoming_score = frame.value.backlog_priority_score();
                if incoming_score <= lowest_score {
                    return IngressDecision::DropBacklog;
                }
                self.queue.remove(lowest_idx);
                self.queue.push_back(frame);
                return IngressDecision::DropBacklog;
            }
            if !frame.value.is_sync_point() {
                return IngressDecision::DropBacklog;
            }
            self.queue.push_back(frame);
            return IngressDecision::DropBacklog;
        }

        self.queue.push_back(frame);
        IngressDecision::Submit
    }

    fn pop(&mut self) -> Option<EncodedFrame> {
        self.queue.pop_front()
    }
}

fn scale_duration_by_per_mille(base: Duration, ratio_per_mille: u16, floor: Duration) -> Duration {
    let scaled_ms = ((base.as_millis() as u128) * ratio_per_mille as u128 / 1_000) as u64;
    Duration::from_millis(scaled_ms).max(floor)
}

#[cfg(test)]
mod tests {
    use super::{FrameScheduler, IngressDecision, VideoIngress};
    use crate::media::video::types::{EncodedFrame, FrameValue, VideoCodec};
    use bytes::Bytes;
    use std::time::{Duration, Instant};

    fn make_frame(
        now: Instant,
        value: FrameValue,
        is_keyframe: bool,
        target_offset_ms: i64,
    ) -> EncodedFrame {
        EncodedFrame {
            codec: VideoCodec::H264,
            is_keyframe,
            config_changed: false,
            value,
            width: 1920,
            height: 1080,
            rtp_timestamp: 1,
            assembled_at: now,
            target_playout_time: if target_offset_ms >= 0 {
                now + Duration::from_millis(target_offset_ms as u64)
            } else {
                now - Duration::from_millis(target_offset_ms.unsigned_abs())
            },
            payload: Bytes::from_static(b"x"),
        }
    }

    #[test]
    fn delta_frame_drops_earlier_than_keyframe() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));

        assert_eq!(
            ingress.submit(
                make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0),
                now
            ),
            IngressDecision::Submit
        );

        let late_delta = make_frame(now, FrameValue::new(false, false, 8 * 1024), false, -150);
        assert_eq!(ingress.submit(late_delta, now), IngressDecision::DropLate);

        let late_keyframe = make_frame(now, FrameValue::new(true, true, 64 * 1024), true, -150);
        assert_eq!(ingress.submit(late_keyframe, now), IngressDecision::Submit);
    }

    #[test]
    fn repeated_config_mismatch_while_waiting_does_not_retrigger_reconfigure() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));

        assert_eq!(
            ingress.submit(
                make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0),
                now
            ),
            IngressDecision::Submit
        );

        let mut mismatched_delta =
            make_frame(now, FrameValue::new(false, false, 8 * 1024), false, 0);
        mismatched_delta.width = 1280;
        mismatched_delta.height = 720;
        assert_eq!(
            ingress.submit(mismatched_delta.clone(), now),
            IngressDecision::Reconfigure
        );
        assert_eq!(
            ingress.submit(mismatched_delta, now),
            IngressDecision::WaitKeyframe
        );
    }

    #[test]
    fn backlog_prefers_higher_value_frame() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(1, Duration::from_millis(250));

        assert_eq!(
            ingress.submit(
                make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0),
                now
            ),
            IngressDecision::Submit
        );

        let refresh_delta = make_frame(now, FrameValue::new(false, true, 6 * 1024), false, 0);
        assert_eq!(
            ingress.submit(refresh_delta, now),
            IngressDecision::DropBacklog
        );
        assert_eq!(ingress.queue_depth(), 1);
        let queued = ingress.pop().expect("frame should remain queued");
        assert!(queued.value.is_sync_point());
    }
}
