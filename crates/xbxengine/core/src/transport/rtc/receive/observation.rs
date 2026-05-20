use super::receiver_state::ReceiverState;

/// 仅用于 diagnostics；不驱动全局 recovery episode。
#[derive(Clone, Debug, Default)]
pub struct ReceiverObservation {
    pub state: ReceiverState,
    pub gap_sequence: Option<u16>,
    pub gap_span: Option<u16>,
    pub nack_in_flight: bool,
    pub keyframe_request_pending: bool,
    pub bootstrap_reject_reason: Option<String>,
}
