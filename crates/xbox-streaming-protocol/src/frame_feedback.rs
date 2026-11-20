//! Xbox input Metadata 沿用原有逐帧报告，清晰度策略只调整发送副本。

const MAX_DECODE_TIME_MS: f64 = 10.0;
pub const XBOX_INPUT_MAX_FRAME_METADATA: usize = 29;
const GAMEPAD_PACKET_BYTES: usize = 38;
const INPUT_HEADER_BYTES: usize = 14;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XboxStreamFrameMetadata {
    pub server_data_key: u32,
    pub arrival_time_ms: f64,
    pub submitted_time_ms: f64,
    pub decoded_time_ms: f64,
    pub rendered_time_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XboxNormalizedFrameMetadata {
    pub wire: XboxStreamFrameMetadata,
    pub raw_decode_time_ms: f64,
    pub adjustment_ms: f64,
}

pub fn normalize_xbox_stream_frame_metadata(
    frame: XboxStreamFrameMetadata,
    prevent_resolution_drops: bool,
) -> Option<XboxNormalizedFrameMetadata> {
    let times = [
        frame.arrival_time_ms,
        frame.submitted_time_ms,
        frame.decoded_time_ms,
        frame.rendered_time_ms,
    ];
    if times.iter().any(|time| !time.is_finite() || *time < 0.0)
        || times.windows(2).any(|pair| pair[1] < pair[0])
    {
        return None;
    }
    let raw_decode_time_ms = frame.decoded_time_ms - frame.submitted_time_ms;
    let adjustment_ms = if prevent_resolution_drops {
        (raw_decode_time_ms - MAX_DECODE_TIME_MS).max(0.0)
    } else {
        0.0
    };
    Some(XboxNormalizedFrameMetadata {
        wire: XboxStreamFrameMetadata {
            decoded_time_ms: frame.decoded_time_ms - adjustment_ms,
            rendered_time_ms: frame.rendered_time_ms - adjustment_ms,
            ..frame
        },
        raw_decode_time_ms,
        adjustment_ms,
    })
}

pub fn append_xbox_stream_input_frame_metadata(
    gamepad_packet: &[u8],
    frames: &[XboxStreamFrameMetadata],
    timestamp_ms: f64,
    prevent_resolution_drops: bool,
) -> Option<Vec<u8>> {
    if gamepad_packet.len() != GAMEPAD_PACKET_BYTES
        || gamepad_packet[..2] != 2u16.to_le_bytes()
        || gamepad_packet[INPUT_HEADER_BYTES] != 1
        || frames.is_empty()
        || frames.len() > XBOX_INPUT_MAX_FRAME_METADATA
        || !timestamp_ms.is_finite()
        || timestamp_ms < 0.0
    {
        return None;
    }
    let normalized = frames
        .iter()
        .copied()
        .map(|frame| {
            (frame.rendered_time_ms <= timestamp_ms)
                .then(|| normalize_xbox_stream_frame_metadata(frame, prevent_resolution_drops))
                .flatten()
        })
        .collect::<Option<Vec<_>>>()?;
    let mut packet = Vec::with_capacity(gamepad_packet.len() + 1 + frames.len() * 28);
    packet.extend_from_slice(&3u16.to_le_bytes());
    packet.extend_from_slice(&gamepad_packet[2..6]);
    packet.extend_from_slice(&timestamp_ms.to_le_bytes());
    packet.push(frames.len() as u8);
    for frame in normalized {
        let values = [
            frame.wire.server_data_key,
            milliseconds_on_wire(frame.wire.arrival_time_ms),
            milliseconds_on_wire(frame.wire.submitted_time_ms),
            milliseconds_on_wire(frame.wire.decoded_time_ms),
            milliseconds_on_wire(frame.wire.rendered_time_ms),
            milliseconds_on_wire(timestamp_ms),
            milliseconds_on_wire(timestamp_ms),
        ];
        for value in values {
            packet.extend_from_slice(&value.to_le_bytes());
        }
    }
    packet.extend_from_slice(&gamepad_packet[INPUT_HEADER_BYTES..]);
    Some(packet)
}

fn milliseconds_on_wire(value: f64) -> u32 {
    value.floor().rem_euclid(4_294_967_296.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(submitted: f64, decoded: f64) -> XboxStreamFrameMetadata {
        XboxStreamFrameMetadata {
            server_data_key: u32::MAX,
            arrival_time_ms: submitted - 1.0,
            submitted_time_ms: submitted,
            decoded_time_ms: decoded,
            rendered_time_ms: decoded + 4.0,
        }
    }

    #[test]
    fn clarity_policy_preserves_observations_and_render_delay_at_thresholds() {
        for decode_ms in [0.0, 5.0, 10.0, 11.0, 32.0] {
            let original = frame(100.0, 100.0 + decode_ms);
            let standard = normalize_xbox_stream_frame_metadata(original, false).unwrap();
            let clarity = normalize_xbox_stream_frame_metadata(original, true).unwrap();
            assert_eq!(standard.wire, original);
            assert_eq!(clarity.raw_decode_time_ms, decode_ms);
            assert_eq!(clarity.wire.decoded_time_ms - 100.0, decode_ms.min(10.0));
            assert_eq!(
                clarity.wire.rendered_time_ms - clarity.wire.decoded_time_ms,
                4.0
            );
            assert_eq!(original.decoded_time_ms, 100.0 + decode_ms);
        }
    }

    #[test]
    fn metadata_precedes_gamepad_and_preserves_sequence_and_release_payload() {
        let mut input = vec![0; GAMEPAD_PACKET_BYTES];
        input[..2].copy_from_slice(&2u16.to_le_bytes());
        input[2..6].copy_from_slice(&u32::MAX.to_le_bytes());
        input[14] = 1;
        input[30..].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let packet =
            append_xbox_stream_input_frame_metadata(&input, &[frame(100.0, 132.0)], 150.0, true)
                .unwrap();
        assert_eq!(packet.len(), 67);
        assert_eq!(&packet[..2], &3u16.to_le_bytes());
        assert_eq!(&packet[2..6], &u32::MAX.to_le_bytes());
        assert_eq!(packet[14], 1);
        assert_eq!(&packet[15..19], &u32::MAX.to_le_bytes());
        assert_eq!(&packet[27..31], &110u32.to_le_bytes());
        assert_eq!(&packet[31..35], &114u32.to_le_bytes());
        assert_eq!(&packet[43..], &input[14..]);
    }

    #[test]
    fn timestamps_wrap_after_adjustment_and_invalid_facts_are_rejected() {
        let boundary = 4_294_967_296.0;
        let normalized =
            normalize_xbox_stream_frame_metadata(frame(boundary - 5.0, boundary + 20.0), true)
                .unwrap();
        assert_eq!(milliseconds_on_wire(normalized.wire.decoded_time_ms), 5);
        assert_eq!(milliseconds_on_wire(normalized.wire.rendered_time_ms), 9);
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 99.0] {
            assert!(normalize_xbox_stream_frame_metadata(frame(100.0, invalid), true).is_none());
        }
        let mut input = vec![0; GAMEPAD_PACKET_BYTES];
        input[..2].copy_from_slice(&2u16.to_le_bytes());
        input[14] = 1;
        assert!(append_xbox_stream_input_frame_metadata(
            &input,
            &[frame(100.0, 132.0)],
            130.0,
            true
        )
        .is_none());
        assert!(append_xbox_stream_input_frame_metadata(
            &input,
            &vec![frame(100.0, 132.0); 30],
            150.0,
            true
        )
        .is_none());
        assert!(
            append_xbox_stream_input_frame_metadata(&[], &[frame(100.0, 132.0)], 150.0, true)
                .is_none()
        );
    }
}
