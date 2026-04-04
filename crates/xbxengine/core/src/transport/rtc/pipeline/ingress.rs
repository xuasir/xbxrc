use std::sync::Arc;

use crate::{
    media::video::{
        decode::actor::DecodeSubmitError,
        ingress::scheduler::{FrameScheduler, VideoIngress},
    },
    runtime_stats_sink::RuntimeStatsSink,
};

use crate::media::video::decode::actor::DecodeActorHandle;

use super::observation::MediaSupervisorObservationState;

pub(super) fn drain_ingress_to_decode(
    ingress: &mut VideoIngress,
    decode_handle: &Arc<DecodeActorHandle>,
    runtime_stats: &RuntimeStatsSink,
    frame_count: u64,
    observation: &MediaSupervisorObservationState,
) {
    loop {
        let demand = decode_handle.demand_snapshot();
        if !demand.accepts_input {
            if demand.pending_output_backpressure && frame_count % 120 == 0 {
                crate::xbx_log_info!(
                    "[MediaSession] decode pending output backpressure, pause ingress handoff (slots={})",
                    demand.available_input_slots
                );
            }
            break;
        }
        let Some(frame) = ingress.pop() else {
            break;
        };
        let frame_w = frame.width;
        let frame_h = frame.height;
        let frame_is_key = frame.is_keyframe;
        let frame_payload_len = frame.payload.len();

        observation.record_stream_dimensions(runtime_stats, frame_w, frame_h);

        if let Err(error) = decode_handle.submit(frame) {
            let frame = match error {
                DecodeSubmitError::Full(frame) | DecodeSubmitError::Disconnected(frame) => frame,
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
                runtime_stats
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
