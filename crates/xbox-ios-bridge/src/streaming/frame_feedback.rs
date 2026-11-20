use super::XboxStreamingError;
use xbox_streaming_protocol::{append_xbox_stream_input_frame_metadata, XboxStreamFrameMetadata};

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxStreamFrameFeedback {
    pub server_data_key: u32,
    pub arrival_time_ms: f64,
    pub submitted_time_ms: f64,
    pub decoded_time_ms: f64,
    pub rendered_time_ms: f64,
}

#[uniffi::export]
pub fn stream_input_frame_feedback_payload(
    gamepad_payload: Vec<u8>,
    frames: Vec<XboxStreamFrameFeedback>,
    timestamp_ms: f64,
    prevent_resolution_drops: bool,
) -> Result<Vec<u8>, XboxStreamingError> {
    let frames = frames
        .into_iter()
        .map(|frame| XboxStreamFrameMetadata {
            server_data_key: frame.server_data_key,
            arrival_time_ms: frame.arrival_time_ms,
            submitted_time_ms: frame.submitted_time_ms,
            decoded_time_ms: frame.decoded_time_ms,
            rendered_time_ms: frame.rendered_time_ms,
        })
        .collect::<Vec<_>>();
    append_xbox_stream_input_frame_metadata(
        &gamepad_payload,
        &frames,
        timestamp_ms,
        prevent_resolution_drops,
    )
    .ok_or_else(|| XboxStreamingError::InvalidArgument("frameFeedbackInvalid".to_string()))
}
