//! 将接收侧事实投影为 `latest_video_timeline_observation` DTO（trace/UI），不含 pre-decode 裁决。

use crate::XbxEngineVideoTimelineObservation;

use super::receiver_state::ReceiverState;
use super::trace_ledger::ReceiverTraceLedger;

pub(crate) fn project_latest_video_timeline_observation(
    receiver_state: ReceiverState,
    trace: &ReceiverTraceLedger,
    nack_observation_id: u64,
    source_event: &str,
    gap_sequence: Option<u16>,
    frame_rtp_timestamp: Option<u32>,
    now_ms: f64,
) -> XbxEngineVideoTimelineObservation {
    trace.snapshot_for_observation_with_receiver_state(
        receiver_state,
        nack_observation_id,
        source_event,
        gap_sequence,
        frame_rtp_timestamp,
        now_ms,
    )
}
