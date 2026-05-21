/// 仅用于 diagnostics；不驱动全局 recovery episode。
#[derive(Clone, Debug, Default)]
pub struct ReceiverObservation {
    pub nack_in_flight: bool,
    pub keyframe_request_pending: bool,
    pub bootstrap_reject_reason: Option<String>,
}
