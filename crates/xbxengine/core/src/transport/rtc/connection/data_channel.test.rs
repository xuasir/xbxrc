use super::data_channel::{
    build_control_video_keyframe_requested_payload, build_input_metadata_packet,
    build_message_handshake_payload, build_post_handshake_message_payloads,
    StreamViewportDimensions, INPUT_CHANNEL_LABEL, MESSAGE_CHANNEL_LABEL,
};

#[test]
fn message_handshake_payload_contains_type_and_version() {
    let payload = build_message_handshake_payload();
    assert!(payload.contains("Handshake"));
    assert!(payload.contains("messageV1"));
}

#[test]
fn input_metadata_packet_has_fixed_header_layout() {
    let packet = build_input_metadata_packet(7, 123.5, 64);
    assert_eq!(packet.len(), 15);
    assert_eq!(u16::from_le_bytes([packet[0], packet[1]]), 8);
    assert_eq!(
        u32::from_le_bytes([packet[2], packet[3], packet[4], packet[5]]),
        7
    );
    assert_eq!(packet[14], 64);
}

#[test]
fn default_channel_labels_are_stable() {
    assert_eq!(MESSAGE_CHANNEL_LABEL, "message");
    assert_eq!(INPUT_CHANNEL_LABEL, "input");
}

#[test]
fn control_video_keyframe_payload_matches_browser_direct_request() {
    let payload = build_control_video_keyframe_requested_payload();
    let value: serde_json::Value = serde_json::from_str(&payload).expect("valid json payload");

    assert_eq!(
        value.get("message").and_then(|v| v.as_str()),
        Some("videoKeyframeRequested")
    );
    assert_eq!(
        value.get("ifrRequested").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn post_handshake_dimensions_follow_runtime_target_resolution() {
    let payloads = build_post_handshake_message_payloads(StreamViewportDimensions {
        width: 2560,
        height: 1440,
    });
    let dimensions = payloads
        .iter()
        .find_map(|payload| {
            let value: serde_json::Value = serde_json::from_str(payload).ok()?;
            if value.get("target").and_then(|v| v.as_str())
                != Some("/streaming/characteristics/dimensionschanged")
            {
                return None;
            }
            value
                .get("content")
                .and_then(|v| v.as_str())
                .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        })
        .expect("dimensionschanged payload");

    assert_eq!(
        dimensions.get("horizontal").and_then(|v| v.as_u64()),
        Some(2560)
    );
    assert_eq!(
        dimensions.get("vertical").and_then(|v| v.as_u64()),
        Some(1440)
    );
    assert_eq!(
        dimensions.get("preferredWidth").and_then(|v| v.as_u64()),
        Some(2560)
    );
    assert_eq!(
        dimensions.get("preferredHeight").and_then(|v| v.as_u64()),
        Some(1440)
    );
    assert_eq!(
        dimensions.get("safeAreaRight").and_then(|v| v.as_u64()),
        Some(2560)
    );
    assert_eq!(
        dimensions.get("safeAreaBottom").and_then(|v| v.as_u64()),
        Some(1440)
    );
    assert_eq!(
        dimensions
            .get("supportsCustomResolution")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}
