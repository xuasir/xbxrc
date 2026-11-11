use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TraceRecordDisposition {
    Enqueued,
    Buffered,
    Filtered,
    QueueFull,
    RowOversized,
    WriterUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TraceFlushStatus {
    Flushed,
    Off,
    Timeout,
    IoError,
    QueueFull,
    WriterUnavailable,
}
