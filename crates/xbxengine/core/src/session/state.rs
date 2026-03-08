#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    Negotiating,
    Primed,
    Running,
    Reconfiguring,
    Recovering,
    Stopped,
}
