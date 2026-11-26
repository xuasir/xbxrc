pub const XBOX_STREAM_INPUT_CHANNEL_LABEL: &str = "input";
pub const XBOX_STREAM_CONTROL_CHANNEL_LABEL: &str = "control";
pub const XBOX_STREAM_CHAT_CHANNEL_LABEL: &str = "chat";
pub const XBOX_STREAM_MESSAGE_CHANNEL_LABEL: &str = "message";
pub const XBOX_STREAM_DEFAULT_VIEWPORT_WIDTH: u32 = 1920;
pub const XBOX_STREAM_DEFAULT_VIEWPORT_HEIGHT: u32 = 1080;

const MESSAGE_HANDSHAKE_ID: &str = "f9c5f412-0e69-4ede-8e62-92c7f5358c56";
const MESSAGE_TRANSACTION_ID: &str = "41f93d5a-900f-4d33-b7a1-2d4ca6747072";
const MESSAGE_CLIENT_APP_INSTALL_ID: &str = "c11ddb2e-c7e3-4f02-a62b-fd5448e0b851";
const CONTROL_ACCESS_KEY: &str = "4BDB3609-C1F1-4195-9B37-FEFF45DA8B8E";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XboxStreamDataChannelProfile {
    pub label: &'static str,
    pub ordered: bool,
    pub protocol_name: &'static str,
}

pub const XBOX_STREAM_DATA_CHANNEL_PROFILES: [XboxStreamDataChannelProfile; 4] = [
    XboxStreamDataChannelProfile {
        label: XBOX_STREAM_INPUT_CHANNEL_LABEL,
        ordered: true,
        protocol_name: "1.0",
    },
    XboxStreamDataChannelProfile {
        label: XBOX_STREAM_CONTROL_CHANNEL_LABEL,
        ordered: true,
        protocol_name: "controlV1",
    },
    XboxStreamDataChannelProfile {
        label: XBOX_STREAM_CHAT_CHANNEL_LABEL,
        ordered: true,
        protocol_name: "chatV1",
    },
    XboxStreamDataChannelProfile {
        label: XBOX_STREAM_MESSAGE_CHANNEL_LABEL,
        ordered: true,
        protocol_name: "messageV1",
    },
];

pub fn build_xbox_stream_message_handshake_payload() -> String {
    serde_json::json!({
        "type": "Handshake",
        "version": "messageV1",
        "id": MESSAGE_HANDSHAKE_ID,
        "cv": "",
    })
    .to_string()
}

pub fn build_xbox_stream_post_handshake_payloads(width: u32, height: u32) -> Vec<String> {
    let width = width.max(1);
    let height = height.max(1);
    vec![
        build_message_payload(
            "/streaming/systemUi/configuration",
            serde_json::json!({
                "version": [8, 0],
                "systemUis": [0],
            }),
        ),
        build_message_payload(
            "/streaming/properties/clientappinstallidchanged",
            serde_json::json!({
                "clientAppInstallId": MESSAGE_CLIENT_APP_INSTALL_ID,
            }),
        ),
        build_message_payload(
            "/streaming/characteristics/orientationchanged",
            serde_json::json!({ "orientation": 0 }),
        ),
        build_message_payload(
            "/streaming/characteristics/touchinputenabledchanged",
            serde_json::json!({ "touchInputEnabled": false }),
        ),
        build_message_payload(
            "/streaming/characteristics/clientdevicecapabilities",
            serde_json::json!({}),
        ),
        build_xbox_stream_dimensions_changed_payload(width, height),
    ]
}

pub fn build_xbox_stream_dimensions_changed_payload(width: u32, height: u32) -> String {
    let width = width.max(1);
    let height = height.max(1);
    build_message_payload(
        "/streaming/characteristics/dimensionschanged",
        serde_json::json!({
            "horizontal": width,
            "vertical": height,
            "preferredWidth": width,
            "preferredHeight": height,
            "safeAreaLeft": 0,
            "safeAreaTop": 0,
            "safeAreaRight": width,
            "safeAreaBottom": height,
            "supportsCustomResolution": true,
        }),
    )
}

pub fn is_xbox_stream_message_handshake_ack(payload: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|json| {
            json.get("type")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .is_some_and(|kind| kind == "HandshakeAck")
}

pub fn build_xbox_stream_control_authorization_payload() -> String {
    serde_json::json!({
        "message": "authorizationRequest",
        "accessKey": CONTROL_ACCESS_KEY,
    })
    .to_string()
}

pub fn build_xbox_stream_control_video_keyframe_requested_payload() -> String {
    serde_json::json!({
        "message": "videoKeyframeRequested",
        "ifrRequested": true,
    })
    .to_string()
}

pub fn build_xbox_stream_control_gamepad_changed_payload(added: bool) -> String {
    serde_json::json!({
        "message": "gamepadChanged",
        "gamepadIndex": 0,
        "wasAdded": added,
    })
    .to_string()
}

pub fn build_xbox_stream_input_metadata_bootstrap_packet(
    time: f64,
    max_touchpoints: u8,
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(15);
    packet.extend_from_slice(&8u16.to_le_bytes());
    packet.extend_from_slice(&0u32.to_le_bytes());
    packet.extend_from_slice(&time.to_le_bytes());
    packet.push(max_touchpoints);
    packet
}

fn build_message_payload(target: &str, content: serde_json::Value) -> String {
    serde_json::json!({
        "type": "Message",
        "content": content.to_string(),
        "id": MESSAGE_TRANSACTION_ID,
        "target": target,
        "cv": "",
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_and_bootstrap_payloads_match_xbox_contract() {
        assert_eq!(XBOX_STREAM_DATA_CHANNEL_PROFILES.len(), 4);
        assert_eq!(
            XBOX_STREAM_DATA_CHANNEL_PROFILES[3].protocol_name,
            "messageV1"
        );
        assert!(build_xbox_stream_message_handshake_payload().contains("messageV1"));
        assert_eq!(
            build_xbox_stream_post_handshake_payloads(1920, 1080).len(),
            6
        );
        assert!(build_xbox_stream_control_authorization_payload().contains("authorizationRequest"));
        assert!(build_xbox_stream_control_video_keyframe_requested_payload()
            .contains("videoKeyframeRequested"));
        assert!(build_xbox_stream_control_gamepad_changed_payload(false)
            .contains(r#""wasAdded":false"#));
        assert_eq!(
            build_xbox_stream_input_metadata_bootstrap_packet(0.0, 64).len(),
            15
        );
    }

    #[test]
    fn handshake_ack_parser_and_dimensions_are_bounded() {
        assert!(is_xbox_stream_message_handshake_ack(
            r#"{"type":"HandshakeAck"}"#
        ));
        assert!(!is_xbox_stream_message_handshake_ack("{}"));
        let payload = build_xbox_stream_dimensions_changed_payload(0, 0);
        assert!(payload.contains(r#"\"horizontal\":1"#));
        assert!(payload.contains(r#"\"vertical\":1"#));
    }
}
