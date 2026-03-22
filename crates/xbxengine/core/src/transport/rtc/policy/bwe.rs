use crate::transport::rtc::bwe::evaluator::RtcBweEvaluation;

#[derive(Clone)]
pub(crate) struct BwePolicyProposal {
    pub(crate) evaluation: RtcBweEvaluation,
}
