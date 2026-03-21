mod answer_adapter;
mod candidate_adapter;
mod offer_policy;
mod types;

pub(crate) use answer_adapter::adapt_remote_answer;
pub(crate) use candidate_adapter::normalize_remote_candidate;
pub(crate) use offer_policy::adapt_local_offer;
pub(crate) use types::RtcSdpContext;
pub(crate) mod policy;
