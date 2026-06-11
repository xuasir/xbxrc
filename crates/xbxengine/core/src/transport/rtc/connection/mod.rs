mod answer_observation;
mod builder;
mod candidate_helpers;
mod control_channel;
mod data_channel;
mod fact_mapping;
mod io_runtime;
mod lifecycle;
mod negotiation;
mod rumble;
mod runtime_state;
mod sdp_candidate;
mod service;
mod text_preview;
mod transport_metrics;
mod turn_runtime;
mod twcc_feedback;

pub(crate) use answer_observation::build_remote_answer_observation;
#[allow(unused_imports)]
pub(crate) use candidate_helpers::dto_to_rtc_candidate;
pub(crate) use candidate_helpers::{
    add_remote_candidate_to_peer, candidate_identity_key, candidate_ip_family,
    classify_candidate_kind, collect_candidate_ip_families, is_end_of_candidates_candidate,
    is_remote_candidate_family_mismatch,
};
#[cfg(test)]
pub(crate) use data_channel::build_control_decoder_reset_payload;

#[cfg(test)]
#[path = "data_channel.test.rs"]
mod data_channel_tests;
pub(crate) use fact_mapping::{map_connection_lifecycle_state_fact, map_data_channel_label_fact};
pub(crate) use sdp_candidate::{
    extract_local_candidates_from_offer_sdp, is_end_of_candidates_marker,
};
pub(crate) use service::{RtcConnectionService, VideoRecoveryRequestOutcome};
pub(crate) use text_preview::short_text_preview;
