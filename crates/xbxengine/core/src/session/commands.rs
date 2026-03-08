#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionCommand {
    RequestKeyframe,
    FlushVideoPipeline,
    ReconfigureVideo,
    RestartTransport,
}
