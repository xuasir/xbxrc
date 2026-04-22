#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionCommand {
    RequestPli,
    FlushVideoPipeline,
    ReconfigureVideo,
    RestartTransport,
}
