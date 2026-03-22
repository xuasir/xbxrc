#[derive(Clone, Debug)]
pub(crate) struct RtcBweEvaluation {
    pub target_remb_kbps: u32,
    pub decision_reason: String,
    pub observation_id: u64,
}
