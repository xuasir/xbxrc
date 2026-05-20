use std::sync::Arc;

use crate::media::video::decode::actor::DecodeActorHandle;
use crate::media::video::ingress::scheduler::VideoIngress;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::ingress::DecodeIngressAdapter;
use crate::transport::rtc::latency::PostDecodeLatencyController;

use super::observation::MediaSupervisorObservationState;

pub(super) fn drain_ingress_to_decode(
    ingress: &mut VideoIngress,
    decode_handle: &Arc<DecodeActorHandle>,
    post_decode: &PostDecodeLatencyController,
    runtime_stats: &RuntimeStatsSink,
    frame_count: u64,
    observation: &MediaSupervisorObservationState,
) {
    DecodeIngressAdapter::new(
        decode_handle.clone(),
        post_decode.clone(),
        runtime_stats.clone(),
    )
    .drain_to_decode(ingress, frame_count, observation);
}
