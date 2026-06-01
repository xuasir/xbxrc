mod core;
mod core_body;
mod core_runtime;
mod decode_gate;
mod decode_gate_eval;
mod engine;
mod feedback_arbiter;
mod h264_bootstrap_tracker;
mod ingress_loop;
mod ingress_state;
mod insert_gate;
mod keyframe_escalation_queue;
mod keyframe_requester;
mod nack_maintenance;
mod nack_policy;
mod nack_requester;
mod observation;
mod packet_buffer;
pub mod pipeline;
mod receiver_state;
pub(crate) mod recovery_ledger;
mod trace_ledger;
pub(crate) use trace_ledger::ReceiverTraceLedger;
mod rtp_frame_assembler;
mod rtx_sink;
pub(crate) mod timeline_projection;
mod timing;

#[cfg(test)]
pub(crate) mod test_fixtures;

#[cfg(test)]
#[path = "ingress_loop.test.rs"]
mod ingress_loop_tests;

#[cfg(test)]
#[path = "rtx_sink.test.rs"]
mod rtx_sink_tests;

#[cfg(test)]
#[path = "timeline_projection.test.rs"]
mod timeline_projection_tests;

pub(crate) use core::receiver_state_from_runtime;
pub(crate) use core_body::ReceiveCoreBody;
pub(crate) use core_runtime::RtcReceiveCore;
pub(crate) use decode_gate::{insert_decodable_to_feed, receiver_decode_context_from_stats};
pub use decode_gate::{
    inspection_bootstrap_blocks_delta_continuation, inspection_bootstrap_reason,
    keyframe_episode_response_detail, receiver_state_blocks_delta_continuation,
    should_block_non_keyframe_admission, DecodeCorruptionPolicy, DecodeGate, DecodeGateDecision,
    InspectionAdmission, ReceiverDecodeContext,
};
pub(crate) use engine::ReceiveEngine;
pub(crate) use insert_gate::{
    insert_decision_label, insert_decision_to_inspection_admission,
    insert_emit_permits_decode_without_bootstrap_ready,
    recovery_keyframe_action_for_insert_decision, resolve_insert_decision_with_reason,
    InsertContext, InsertDecision,
};
pub use observation::ReceiverObservation;
pub use packet_buffer::SequenceObserveOutcome;
pub(crate) use pipeline::build_rtc_receive_pipeline;
pub(crate) use receiver_state::ReceiverState;
pub use rtp_frame_assembler::{RtpAccessUnit, SyntheticMarkerBoundary};

pub(crate) use ingress_state::{
    build_rtc_video_frame_source, now_ms_f64, RtcVideoFrameSource,
    RtcVideoTransportObservationSource, UINT16SIZE_HALF,
};

#[cfg(test)]
pub(crate) use ingress_state::{test_nack_scheduler_config, test_transport_capability};
