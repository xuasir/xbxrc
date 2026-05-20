use std::sync::Arc;
use std::time::Instant;

use crate::media::video::decode::actor::{DecodeActorHandle, DecodeSubmitError};
use crate::media::video::ingress::scheduler::{FrameScheduler, VideoIngress};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::latency::PostDecodeLatencyController;
use crate::transport::rtc::pipeline::observation::MediaSupervisorObservationState;

/// decode 前与 decode 后边界：把 ingress backlog 提交给 decode actor。
pub struct DecodeIngressAdapter {
    decode_handle: Arc<DecodeActorHandle>,
    post_decode: PostDecodeLatencyController,
    runtime_stats: RuntimeStatsSink,
}

impl DecodeIngressAdapter {
    pub fn new(
        decode_handle: Arc<DecodeActorHandle>,
        post_decode: PostDecodeLatencyController,
        runtime_stats: RuntimeStatsSink,
    ) -> Self {
        Self {
            decode_handle,
            post_decode,
            runtime_stats,
        }
    }

    pub fn drain_to_decode(
        &self,
        ingress: &mut VideoIngress,
        frame_count: u64,
        observation: &MediaSupervisorObservationState,
    ) {
        let host_stall_throttle = self.post_decode.host_stall_throttle_enabled();
        let now = Instant::now();
        let _ = ingress.drain_expired_for_decode(now);
        loop {
            let demand = self.decode_handle.demand_snapshot();
            if !demand.accepts_input {
                if demand.pending_output_backpressure && frame_count % 120 == 0 {
                    crate::xbx_log_info!(
                        "[MediaSession] decode pending output backpressure, pause ingress handoff (slots={})",
                        demand.available_input_slots
                    );
                }
                break;
            }
            if host_stall_throttle {
                // 队头非关键帧会阻塞后续 IDR 入解码邮箱；有界丢弃前缀，避免把节流变成主动卡死。
                const MAX_HOST_STALL_HEAD_DISCARDS: usize = 512;
                ingress.discard_non_keyframe_prefix_for_host_stall(MAX_HOST_STALL_HEAD_DISCARDS);
                match ingress.peek_front() {
                    None => break,
                    Some(front) if !front.is_keyframe => break,
                    _ => {}
                }
            }
            let Some(frame) = ingress.pop() else {
                break;
            };
            let frame_w = frame.width;
            let frame_h = frame.height;
            let frame_is_key = frame.is_keyframe;
            let frame_payload_len = frame.payload.len();

            observation.record_stream_dimensions(&self.runtime_stats, frame_w, frame_h);

            if let Err(error) = self.decode_handle.submit(frame) {
                let frame = match error {
                    DecodeSubmitError::Full(frame) | DecodeSubmitError::Disconnected(frame) => {
                        frame
                    }
                };
                ingress.requeue_front(frame);
                crate::xbx_log_warn!(
                    "[MediaSession] decode queue unavailable, keep remaining ingress backlog"
                );
                break;
            }

            let frame_type = if frame_is_key { "KEYFRAME" } else { "DELTA" };
            if frame_is_key || frame_count % 300 == 0 {
                let (w, h) = if frame_w > 0 {
                    (frame_w, frame_h)
                } else {
                    self.runtime_stats
                        .read(|stats| {
                            stats
                                .latest_video_stream_width
                                .zip(stats.latest_video_stream_height)
                        })
                        .flatten()
                        .unwrap_or((0, 0))
                };
                crate::xbx_log_info!(
                    "[MediaSession] received valid {} frame (size: {}B, res: {}x{}, frame#: {})",
                    frame_type,
                    frame_payload_len,
                    w,
                    h,
                    frame_count
                );
            }
        }
    }
}
