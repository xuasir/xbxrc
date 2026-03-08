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
    waiting_keyframe: bool,
    current_width: u32,
    current_height: u32,
    current_codec: Option<VideoCodec>,
}

impl VideoIngress {
    pub fn new(max_size: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(max_size),
            max_size,
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
}

impl FrameScheduler for VideoIngress {
    fn submit(&mut self, frame: EncodedFrame, now: Instant) -> IngressDecision {
        if frame.config_changed {
            self.start_reconfigure();
            return IngressDecision::Reconfigure;
        }

        // 基础参数变化也会触发必须使用 Keyframe 初始化
        let config_mismatch = self.current_codec.as_ref() != Some(&frame.codec)
            || self.current_width != frame.width
            || self.current_height != frame.height;

        if config_mismatch && !frame.is_keyframe {
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

        // Rule 2: Delta 晚到即丢弃 (now > target_playout + 500ms)
        // 放宽限制：低延迟场景宁可播晚帧也不愿意频繁断流等待关键帧。
        if now > frame.target_playout_time + Duration::from_millis(500) {
            crate::xbx_log_warn!(
                "[VideoIngress] frame too late, dropping. now={:?}, target={:?}",
                now,
                frame.target_playout_time
            );
            return IngressDecision::DropLate;
        }

        // Rule 3: Backlog 控制
        if self.queue.len() >= self.max_size {
            // drop oldest delta
            self.queue.pop_front();
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
