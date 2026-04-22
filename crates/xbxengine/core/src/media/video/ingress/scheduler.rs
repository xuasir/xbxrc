//! Ingress 调度：`IngressDecision` 与 `VideoIngress`。
//! RFC：准入与本地丢弃决策归属本层；禁止输出 transport 级 `TransportRecover` 决策。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::{EncodedFrame, FrameRecoveryDisposition, VideoCodec};
use crate::transport::rtc::recovery::contract::is_recovery_delta_continuation_ready;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngressDecision {
    Submit,
    DropLate,
    DropBacklogIncoming,
    DropBacklogEvictQueued,
    DropUnrecoverable,
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
    ingress_awaiting_bootstrap: bool,
    observed_width: u32,
    observed_height: u32,
    observed_codec: Option<VideoCodec>,
    committed_width: u32,
    committed_height: u32,
    committed_codec: Option<VideoCodec>,
}

impl VideoIngress {
    pub fn new(max_size: usize, late_frame_drop_threshold: Duration) -> Self {
        Self {
            queue: VecDeque::with_capacity(max_size),
            max_size,
            late_frame_drop_threshold,
            ingress_awaiting_bootstrap: true,
            observed_width: 0,
            observed_height: 0,
            observed_codec: None,
            committed_width: 0,
            committed_height: 0,
            committed_codec: None,
        }
    }

    /// 主动清除队列，进入重配状态直到收到 Keyframe
    pub fn start_reconfigure(&mut self) {
        self.queue.clear();
        self.ingress_awaiting_bootstrap = true;
    }

    pub fn drain_expired_for_decode(&mut self, now: Instant) -> usize {
        self.drain_expired_queued_frames(now, "decodePull")
    }

    pub fn queue_depth(&self) -> usize {
        self.queue.len()
    }

    pub fn peek_front(&self) -> Option<&EncodedFrame> {
        self.queue.front()
    }

    /// Host present 解码节流：丢弃队首连续非关键帧（最多 `max_total`），避免队头 delta 挡住后续 IDR（HOL）。
    pub fn discard_non_keyframe_prefix_for_host_stall(&mut self, max_total: usize) -> usize {
        let mut discarded = 0usize;
        while discarded < max_total {
            match self.queue.front() {
                None => break,
                Some(f) if f.is_keyframe => break,
                Some(_) => {
                    self.queue.pop_front();
                    discarded += 1;
                }
            }
        }
        discarded
    }

    /// decode handoff 失败时，把已出队的帧放回 ingress 头部，等待下一次 budget 窗口。
    pub fn requeue_front(&mut self, frame: EncodedFrame) {
        self.queue.push_front(frame);
    }

    /// 返回这帧若触发 reconfigure 时的具体原因，便于把 trace 收敛到参数集/尺寸/codec 维度。
    pub fn describe_reconfigure_reason(&self, frame: &EncodedFrame) -> Option<String> {
        let codec_changed = self
            .committed_codec
            .as_ref()
            .is_some_and(|codec| codec != &frame.codec);
        let dimensions_changed = self.committed_width != 0
            && self.committed_height != 0
            && (self.committed_width != frame.width || self.committed_height != frame.height);

        let mut reasons = Vec::new();
        if frame.h264.parameter_sets_changed {
            reasons.push("parameterSetsChanged");
        }
        if dimensions_changed {
            reasons.push("dimensionsChanged");
        }
        if codec_changed {
            reasons.push("codecChanged");
        }

        if frame.config_changed {
            if reasons.is_empty() {
                reasons.push("configChanged");
            }
            return Some(reasons.join(","));
        }

        if reasons.is_empty() {
            return None;
        }

        Some(reasons.join(","))
    }

    fn drain_expired_queued_frames(&mut self, now: Instant, source: &'static str) -> usize {
        let mut drained = 0usize;
        while self.queue.front().is_some_and(|frame| {
            Self::is_frame_too_late(frame, now, self.late_frame_drop_threshold)
        }) {
            self.queue.pop_front();
            drained = drained.saturating_add(1);
        }
        if drained > 0 {
            crate::xbx_log_debug!(
                "[VideoIngress] drained expired queued frames count={} source={}",
                drained,
                source
            );
        }
        drained
    }

    fn observe_stream_params(&mut self, frame: &EncodedFrame) {
        self.observed_codec = Some(frame.codec.clone());
        self.observed_width = frame.width;
        self.observed_height = frame.height;
    }

    fn commit_stream_params(&mut self, frame: &EncodedFrame) {
        self.committed_codec = Some(frame.codec.clone());
        self.committed_width = frame.width;
        self.committed_height = frame.height;
        self.observe_stream_params(frame);
    }

    fn is_frame_too_late(frame: &EncodedFrame, now: Instant, base_threshold: Duration) -> bool {
        let frame_late_threshold = scale_duration_by_per_mille(
            base_threshold,
            frame.budget.late_budget_ratio_per_mille(frame.value),
            Duration::from_millis(33),
        );
        now > frame.target_playout_instant + frame_late_threshold
    }

    fn can_exit_waiting_keyframe_with_recovery_continuation(
        &self,
        frame: &EncodedFrame,
        config_mismatch: bool,
    ) -> bool {
        // 冷启动时 committed_* 为空，config_mismatch 必然为 true，此路径不可能触发。
        // 只有 bootstrap_ready IDR 建立 committed 参数集后，后续 delta 才能走这条路。
        // 注意：原来还允许 `bootstrap_ready=false` 的 IDR 走此路径，现已删除——
        // 这类帧会在上游 `is_keyframe` 分支（步骤 7）处理，返回 WaitKeyframe，行为不变。
        if !self.ingress_awaiting_bootstrap || config_mismatch || frame.config_changed {
            return false;
        }
        if self.committed_codec.as_ref() != Some(&frame.codec)
            || self.committed_width == 0
            || self.committed_height == 0
        {
            return false;
        }
        if frame.h264.parameter_sets_changed {
            return false;
        }
        if !is_recovery_delta_continuation_ready(&frame.h264) {
            return false;
        }
        true
    }
}

impl FrameScheduler for VideoIngress {
    fn submit(&mut self, frame: EncodedFrame, now: Instant) -> IngressDecision {
        let config_mismatch = self.committed_codec.as_ref() != Some(&frame.codec)
            || self.committed_width != frame.width
            || self.committed_height != frame.height;
        let mut context = frame.budget;
        if matches!(
            frame.frame_recovery_disposition,
            FrameRecoveryDisposition::UnrecoverableReferenceChain
        ) {
            context = FrameBudgetContext::for_ingress_admission(&frame, true, config_mismatch);
        } else if self.ingress_awaiting_bootstrap {
            context = FrameBudgetContext::for_ingress_admission(&frame, true, config_mismatch);
        } else if config_mismatch || frame.config_changed || frame.h264.parameter_sets_changed {
            context = FrameBudgetContext::for_ingress_admission(&frame, false, true);
        }
        let disposition = frame.frame_recovery_disposition;
        if matches!(
            disposition,
            FrameRecoveryDisposition::UnrecoverableReferenceChain
        ) || context.prefers_chain_broken()
        {
            // 参考链已污染时，直接前置放弃并等待后续 keyframe 重建。
            self.queue.clear();
            self.ingress_awaiting_bootstrap = true;
            return IngressDecision::DropUnrecoverable;
        }
        if matches!(disposition, FrameRecoveryDisposition::UnrecoverableLate) {
            return IngressDecision::DropUnrecoverable;
        }

        // 冷启动仍坚持 clean bootstrap；恢复期则允许在"已有 committed 参数集 +
        // continuation 可承接"的前提下先退出硬等待，优先保活恢复链。
        if frame.h264.bootstrap_ready {
            self.commit_stream_params(&frame);
            self.ingress_awaiting_bootstrap = false;
            frame.h264.commit();

            // 永远优先: 清空 backlog
            self.queue.clear();
            self.queue.push_back(frame);
            return IngressDecision::Submit;
        }

        if self.can_exit_waiting_keyframe_with_recovery_continuation(&frame, config_mismatch) {
            self.commit_stream_params(&frame);
            // continuation 放行即承认当前流参数可用，commit 后 ingress_awaiting_bootstrap 退出。
            // 冷启动下 committed_* 为空导致 config_mismatch=true，此分支不可能触发，
            // 因此 commit_stream_params 不会在没有 committed 基准的情况下被调用。
            self.ingress_awaiting_bootstrap = false;
            frame.h264.commit();
            self.queue.clear();
            self.queue.push_back(frame);
            return IngressDecision::Submit;
        }

        if frame.is_keyframe {
            self.observe_stream_params(&frame);

            if self.ingress_awaiting_bootstrap {
                return IngressDecision::WaitKeyframe;
            }

            self.commit_stream_params(&frame);
            frame.h264.commit();
            self.queue.clear();
            self.queue.push_back(frame);
            return IngressDecision::Submit;
        }

        if frame.config_changed {
            if context.prefers_wait_keyframe() {
                return IngressDecision::WaitKeyframe;
            }
            if context.prefers_reconfigure() {
                self.start_reconfigure();
                return IngressDecision::Reconfigure;
            }
        }

        // 基础参数变化也会触发必须使用 Keyframe 初始化
        if config_mismatch {
            if context.prefers_wait_keyframe() {
                return IngressDecision::WaitKeyframe;
            }
            if context.prefers_reconfigure() {
                self.start_reconfigure();
                return IngressDecision::Reconfigure;
            }
        }

        // 丢弃期间等待关键帧
        if context.prefers_wait_keyframe() {
            return IngressDecision::WaitKeyframe;
        }

        if Self::is_frame_too_late(&frame, now, self.late_frame_drop_threshold) {
            crate::xbx_log_warn!(
                "[VideoIngress] frame too late, dropping. now={:?}, target={:?}",
                now,
                frame.target_playout_instant
            );
            return IngressDecision::DropLate;
        }

        // 先清掉已失效的旧帧，避免它们继续占住 ingress backlog 扩大尾延迟。
        self.drain_expired_queued_frames(now, "submit");

        // Rule 3: Backlog 控制
        if self.queue.len() >= self.max_size {
            // backlog 时基于帧价值模型做替换，避免低价值旧帧继续堵塞队列。
            if let Some((lowest_idx, lowest_score)) = self
                .queue
                .iter()
                .enumerate()
                .map(|(idx, queued)| {
                    let queued_context = if self.committed_codec.as_ref() != Some(&queued.codec)
                        || self.committed_width != queued.width
                        || self.committed_height != queued.height
                    {
                        FrameBudgetContext::for_ingress_admission(queued, false, true)
                    } else {
                        queued.budget
                    };
                    (idx, queued_context.backlog_priority_score(queued.value))
                })
                .min_by_key(|(_, score)| *score)
            {
                let incoming_score = context.backlog_priority_score(frame.value);
                if incoming_score <= lowest_score {
                    return IngressDecision::DropBacklogIncoming;
                }
                self.queue.remove(lowest_idx);
                frame.h264.commit();
                self.queue.push_back(frame);
                return IngressDecision::DropBacklogEvictQueued;
            }
            if !frame.value.is_sync_point() {
                return IngressDecision::DropBacklogIncoming;
            }
            frame.h264.commit();
            self.queue.push_back(frame);
            return IngressDecision::DropBacklogEvictQueued;
        }

        frame.h264.commit();
        self.queue.push_back(frame);
        IngressDecision::Submit
    }

    fn pop(&mut self) -> Option<EncodedFrame> {
        self.queue.pop_front()
    }
}

#[cfg(test)]
impl VideoIngress {
    /// 单测注入队列形态（不经 `submit` 完整语义），用于节流/HOL 等纯队列行为。
    pub fn test_push_back_unchecked(&mut self, frame: EncodedFrame) {
        self.queue.push_back(frame);
    }
}

fn scale_duration_by_per_mille(base: Duration, ratio_per_mille: u16, floor: Duration) -> Duration {
    let scaled_ms = ((base.as_millis() as u128) * ratio_per_mille as u128 / 1_000) as u64;
    Duration::from_millis(scaled_ms).max(floor)
}

#[cfg(test)]
mod tests {
    use super::{FrameScheduler, IngressDecision, VideoIngress};
    use crate::media::video::h264::inspection::{
        H264AccessUnitInspection, H264AccessUnitInspector, H264BootstrapRejectReason, H264NalUnit,
    };
    use crate::media::video::ingress::budget::FrameBudgetContext;
    use crate::media::video::test_fixtures::{bootstrap_pps_nalu, bootstrap_sps_nalu};
    use crate::media::video::types::{
        EncodedFrame, FrameRecoveryDisposition, FrameValue, VideoCodec,
    };
    use bytes::Bytes;
    use h264_reader::nal::UnitType;

    use std::time::{Duration, Instant};

    fn make_h264_inspection(bootstrap_ready: bool) -> H264AccessUnitInspection {
        let commit_state = if bootstrap_ready {
            let inspector = H264AccessUnitInspector::new();
            inspector
                .seed_committed_parameter_sets_if_absent(bootstrap_sps_nalu(), bootstrap_pps_nalu())
                .expect("fixture sps/pps seed");
            inspector.shared_commit_state()
        } else {
            H264AccessUnitInspector::test_commit_state()
        };
        H264AccessUnitInspection {
            nals: Vec::new(),
            parameter_sets: None,
            width: Some(1920),
            height: Some(1080),
            is_idr: bootstrap_ready,
            has_inband_sps: bootstrap_ready,
            has_inband_pps: bootstrap_ready,
            slice_headers_valid: bootstrap_ready,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready,
            bootstrap_reject_reason: if bootstrap_ready {
                None
            } else {
                Some(H264BootstrapRejectReason::MissingSps)
            },
            commit_state,
        }
    }

    fn make_h264_inspection_with_commit_state(
        template: &H264AccessUnitInspection,
        bootstrap_ready: bool,
        is_idr: bool,
    ) -> H264AccessUnitInspection {
        H264AccessUnitInspection {
            nals: Vec::new(),
            parameter_sets: None,
            width: Some(1920),
            height: Some(1080),
            is_idr,
            has_inband_sps: bootstrap_ready,
            has_inband_pps: bootstrap_ready,
            slice_headers_valid: true,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready,
            bootstrap_reject_reason: if bootstrap_ready {
                None
            } else {
                Some(H264BootstrapRejectReason::NonIdrVcl)
            },
            commit_state: template.commit_state.clone(),
        }
    }

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
            budget: FrameBudgetContext::steady_for_value(value),
            width: 1920,
            height: 1080,
            rtp_timestamp: 1,
            first_packet_sequence: None,
            frame_playout_deadline_at_ms: None,
            frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            target_playout_instant: if target_offset_ms >= 0 {
                now + Duration::from_millis(target_offset_ms as u64)
            } else {
                now - Duration::from_millis(target_offset_ms.unsigned_abs())
            },
            h264: make_h264_inspection(is_keyframe),
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
            IngressDecision::DropBacklogIncoming
        );
        assert_eq!(ingress.queue_depth(), 1);
        let queued = ingress.pop().expect("frame should remain queued");
        assert!(queued.value.is_sync_point());
    }

    #[test]
    fn backlog_evict_reports_distinct_decision() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(1, Duration::from_millis(250));
        assert_eq!(
            ingress.submit(
                make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0),
                now
            ),
            IngressDecision::Submit
        );

        let mut low_value = ingress.pop().expect("seed frame");
        low_value.value = FrameValue::new(false, false, 2 * 1024);
        ingress.test_push_back_unchecked(low_value);
        let incoming_high = make_frame(now, FrameValue::new(true, false, 32 * 1024), false, 5);
        assert_eq!(
            ingress.submit(incoming_high, now),
            IngressDecision::DropBacklogEvictQueued
        );
    }

    #[test]
    fn expired_backlog_frames_are_drained_before_admitting_fresh_frame() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(2, Duration::from_millis(250));

        assert_eq!(
            ingress.submit(
                make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0),
                now
            ),
            IngressDecision::Submit
        );

        let stale_delta = make_frame(now, FrameValue::new(false, false, 8 * 1024), false, -500);
        assert_eq!(ingress.submit(stale_delta, now), IngressDecision::DropLate);

        let much_later = now + Duration::from_millis(500);
        let fresh_delta = make_frame(
            much_later,
            FrameValue::new(false, true, 6 * 1024),
            false,
            20,
        );
        assert_eq!(
            ingress.submit(fresh_delta, much_later),
            IngressDecision::Submit
        );
        assert_eq!(ingress.queue_depth(), 1);
        let queued = ingress.pop().expect("fresh frame should remain queued");
        assert_eq!(
            queued.target_playout_instant,
            much_later + Duration::from_millis(20)
        );
    }

    #[test]
    fn config_changed_keyframe_while_waiting_is_still_accepted() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));

        let mut first_keyframe = make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0);
        first_keyframe.config_changed = true;
        first_keyframe.width = 2560;
        first_keyframe.height = 1440;

        assert_eq!(ingress.submit(first_keyframe, now), IngressDecision::Submit);
        assert_eq!(ingress.queue_depth(), 1);

        let queued = ingress.pop().expect("keyframe should be queued");
        assert!(queued.is_keyframe);
        assert!(queued.config_changed);
        assert_eq!(queued.width, 2560);
        assert_eq!(queued.height, 1440);
    }

    #[test]
    fn waiting_keyframe_requires_clean_bootstrap_frame() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));

        let dirty_keyframe = EncodedFrame {
            h264: make_h264_inspection(false),
            ..make_frame(now, FrameValue::new(true, false, 64 * 1024), true, 0)
        };
        assert_eq!(
            ingress.submit(dirty_keyframe, now),
            IngressDecision::WaitKeyframe
        );
    }

    #[test]
    fn recovery_continuation_can_exit_waiting_keyframe_after_committed_bootstrap() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));

        let clean_bootstrap = make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0);
        assert_eq!(
            ingress.submit(clean_bootstrap.clone(), now),
            IngressDecision::Submit
        );
        let _ = ingress.pop();

        ingress.start_reconfigure();

        let mut continuation_h264 =
            make_h264_inspection_with_commit_state(&clean_bootstrap.h264, false, false);
        // `delta_continuation_ready` 要求存在 VCL NAL；仅测 ingress 出口，占位一条即可。
        continuation_h264.nals.push(H264NalUnit {
            range: 0..0,
            unit_type: UnitType::SliceLayerWithoutPartitioningNonIdr,
        });
        let continuation = EncodedFrame {
            h264: continuation_h264,
            ..make_frame(now, FrameValue::new(false, true, 8 * 1024), false, 0)
        };
        assert_eq!(continuation.h264.committed_sps_present(), true);
        assert_eq!(continuation.h264.committed_pps_present(), true);
        assert_eq!(continuation.h264.delta_continuation_ready(), true);
        assert_eq!(ingress.submit(continuation, now), IngressDecision::Submit);
        assert_eq!(ingress.queue_depth(), 1);
    }

    #[test]
    fn cold_start_delta_without_committed_context_stays_in_wait_keyframe() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));

        let delta = EncodedFrame {
            h264: make_h264_inspection(false),
            ..make_frame(now, FrameValue::new(false, true, 8 * 1024), false, 0)
        };
        assert!(!delta.h264.committed_sps_present());
        assert!(!delta.h264.committed_pps_present());
        assert!(!delta.h264.delta_continuation_ready());
        assert_eq!(ingress.submit(delta, now), IngressDecision::WaitKeyframe);
    }

    #[test]
    fn dirty_keyframe_updates_observed_but_not_committed_state() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));
        let bootstrap = make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0);
        assert_eq!(ingress.submit(bootstrap, now), IngressDecision::Submit);
        ingress.start_reconfigure();

        let mut dirty_keyframe = make_frame(now, FrameValue::new(true, false, 64 * 1024), true, 0);
        dirty_keyframe.h264 = make_h264_inspection(false);
        dirty_keyframe.width = 2560;
        dirty_keyframe.height = 1440;
        assert_eq!(
            ingress.submit(dirty_keyframe, now),
            IngressDecision::WaitKeyframe
        );
        let delta = make_frame(now, FrameValue::new(false, true, 8 * 1024), false, 0);
        assert_eq!(ingress.submit(delta, now), IngressDecision::WaitKeyframe);
    }

    #[test]
    fn describe_reconfigure_reason_prefers_parameter_sets_dimensions_and_codec() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));

        assert_eq!(
            ingress.submit(
                make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0),
                now
            ),
            IngressDecision::Submit
        );

        let mut changed = make_frame(now, FrameValue::new(false, false, 8 * 1024), false, 0);
        changed.width = 2560;
        changed.height = 1440;
        changed.config_changed = true;
        changed.h264.parameter_sets_changed = true;

        assert_eq!(
            ingress.describe_reconfigure_reason(&changed).as_deref(),
            Some("parameterSetsChanged,dimensionsChanged")
        );
    }

    #[test]
    fn unrecoverable_late_frame_is_dropped_before_decode() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));
        let mut delta = make_frame(now, FrameValue::new(false, false, 8 * 1024), false, 0);
        delta.frame_recovery_disposition = FrameRecoveryDisposition::UnrecoverableLate;
        delta.frame_unrecoverable_reason = Some("deadline".to_string());
        assert_eq!(
            ingress.submit(delta, now),
            IngressDecision::DropUnrecoverable
        );
        assert_eq!(ingress.queue_depth(), 0);
    }

    #[test]
    fn unrecoverable_reference_chain_enters_wait_keyframe_path() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(4, Duration::from_millis(250));
        assert_eq!(
            ingress.submit(
                make_frame(now, FrameValue::new(true, true, 64 * 1024), true, 0),
                now
            ),
            IngressDecision::Submit
        );
        let mut reference_delta = make_frame(now, FrameValue::new(false, true, 8 * 1024), false, 0);
        reference_delta.frame_recovery_disposition =
            FrameRecoveryDisposition::UnrecoverableReferenceChain;
        reference_delta.frame_unrecoverable_reason = Some("referenceChain".to_string());
        assert_eq!(
            ingress.submit(reference_delta, now),
            IngressDecision::DropUnrecoverable
        );
        let next_delta = make_frame(now, FrameValue::new(false, false, 8 * 1024), false, 0);
        assert_eq!(
            ingress.submit(next_delta, now),
            IngressDecision::WaitKeyframe
        );
    }

    #[test]
    fn discard_non_keyframe_prefix_for_host_stall_reveals_keyframe() {
        let now = Instant::now();
        let mut ingress = VideoIngress::new(8, Duration::from_millis(250));
        ingress.test_push_back_unchecked(make_frame(
            now,
            FrameValue::new(false, false, 8 * 1024),
            false,
            0,
        ));
        ingress.test_push_back_unchecked(make_frame(
            now,
            FrameValue::new(false, false, 8 * 1024),
            false,
            10,
        ));
        ingress.test_push_back_unchecked(make_frame(
            now,
            FrameValue::new(true, true, 64 * 1024),
            true,
            20,
        ));
        assert_eq!(ingress.discard_non_keyframe_prefix_for_host_stall(512), 2);
        let popped = FrameScheduler::pop(&mut ingress).expect("keyframe after discard");
        assert!(popped.is_keyframe);
    }
}
