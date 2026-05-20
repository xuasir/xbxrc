/// receiver-local 四态：不表达跨层 display/host 语义。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReceiverState {
    #[default]
    Priming,
    Receiving,
    Repairing,
    WaitingKeyframe,
}

impl ReceiverState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Priming => "priming",
            Self::Receiving => "receiving",
            Self::Repairing => "repairing",
            Self::WaitingKeyframe => "waiting-keyframe",
        }
    }

    /// trace DTO `chain.state` 口径（RFC 四态，与 [`as_str`] 一致）。
    pub(crate) fn timeline_chain_state_label(self) -> &'static str {
        self.as_str()
    }

    pub(crate) fn timeline_chain_reason(self) -> Option<&'static str> {
        match self {
            Self::WaitingKeyframe => Some("receiverWaitingKeyframe"),
            Self::Repairing => Some("gapRepairInFlight"),
            _ => None,
        }
    }
}
