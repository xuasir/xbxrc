use super::data_channel::{
    build_control_video_keyframe_requested_payload, build_input_metadata_packet,
    build_message_handshake_payload, INPUT_CHANNEL_LABEL, MESSAGE_CHANNEL_LABEL,
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
