#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    PacketLossBurst,
    FrameReady,
    DecodeError,
    ResolutionChanged,
    KeyframeReceived,
}
