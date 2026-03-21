use rtc::data_channel::RTCDataChannelInit;
use rtc::peer_connection::RTCPeerConnection;

use super::runtime_state::RtcConnectionRuntimeState;
use crate::transport::rtc::stats::now_ms_f64;
use crate::XbxEngineRuntimeError;

pub(crate) const MESSAGE_CHANNEL_LABEL: &str = "message";
pub(crate) const CONTROL_CHANNEL_LABEL: &str = "control";
pub(crate) const INPUT_CHANNEL_LABEL: &str = "input";
pub(crate) const CHAT_CHANNEL_LABEL: &str = "chat";

const MESSAGE_HANDSHAKE_ID: &str = "f9c5f412-0e69-4ede-8e62-92c7f5358c56";
const MESSAGE_TRANSACTION_ID: &str = "41f93d5a-900f-4d33-b7a1-2d4ca6747072";
const MESSAGE_CLIENT_APP_INSTALL_ID: &str = "c11ddb2e-c7e3-4f02-a62b-fd5448e0b851";
const CONTROL_ACCESS_KEY: &str = "4BDB3609-C1F1-4195-9B37-FEFF45DA8B8E";
const DEFAULT_VIEWPORT_WIDTH: u32 = 1920;
const DEFAULT_VIEWPORT_HEIGHT: u32 = 1080;
const INPUT_METADATA_SEQ: u32 = 0;
const INPUT_METADATA_MAX_TOUCHPOINTS: u8 = 64;

// phase-1 先只把控制面 channel 拓扑建进 rtc，真正的 ready/handshake 由后续事件循环接管。
pub(crate) fn bootstrap_default_channels(
    peer_connection: &mut RTCPeerConnection,
    state: &mut RtcConnectionRuntimeState,
) -> Result<(), XbxEngineRuntimeError> {
    for (label, ordered, protocol) in [
        (INPUT_CHANNEL_LABEL, true, "1.0"),
        (CONTROL_CHANNEL_LABEL, true, "controlV1"),
        (CHAT_CHANNEL_LABEL, true, "chatV1"),
        (MESSAGE_CHANNEL_LABEL, true, "messageV1"),
    ] {
        let channel = peer_connection
            .create_data_channel(
                label,
                Some(RTCDataChannelInit {
                    ordered,
                    protocol: protocol.to_string(),
                    ..Default::default()
                }),
            )
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!(
                    "xbxEngineRtcCreateDataChannelFailed({label}): {err}"
                ))
            })?;
        state
            .data_channel_labels
            .insert(channel.id(), channel.label().to_string());
    }
    Ok(())
}

pub(crate) fn build_message_handshake_payload() -> String {
    serde_json::json!({
        "type": "Handshake",
        "version": "messageV1",
        "id": MESSAGE_HANDSHAKE_ID,
        "cv": "",
    })
    .to_string()
}

pub(crate) fn build_post_handshake_message_payloads() -> Vec<String> {
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
            serde_json::json!({
                "orientation": 0,
            }),
        ),
        build_message_payload(
            "/streaming/characteristics/touchinputenabledchanged",
            serde_json::json!({
                "touchInputEnabled": false,
            }),
        ),
        build_message_payload(
            "/streaming/characteristics/clientdevicecapabilities",
            serde_json::json!({}),
        ),
        build_message_payload(
            "/streaming/characteristics/dimensionschanged",
            serde_json::json!({
                "horizontal": DEFAULT_VIEWPORT_WIDTH,
                "vertical": DEFAULT_VIEWPORT_HEIGHT,
                "preferredWidth": DEFAULT_VIEWPORT_WIDTH,
                "preferredHeight": DEFAULT_VIEWPORT_HEIGHT,
                "safeAreaLeft": 0,
                "safeAreaTop": 0,
                "safeAreaRight": DEFAULT_VIEWPORT_WIDTH,
                "safeAreaBottom": DEFAULT_VIEWPORT_HEIGHT,
                "supportsCustomResolution": true,
            }),
        ),
    ]
}

pub(crate) fn is_handshake_ack_payload(payload: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|json| {
            json.get("type")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .is_some_and(|kind| kind == "HandshakeAck")
}

pub(crate) fn build_control_keyframe_request_payload() -> String {
    serde_json::json!({
        "message": "videoKeyframeRequested",
        "ifrRequested": true,
    })
    .to_string()
}

pub(crate) fn build_control_decoder_reset_payload() -> String {
    serde_json::json!({
        "message": "decoderReset",
    })
    .to_string()
}

pub(crate) fn build_control_authorization_payload() -> String {
    serde_json::json!({
        "message": "authorizationRequest",
        "accessKey": CONTROL_ACCESS_KEY,
    })
    .to_string()
}

pub(crate) fn build_control_gamepad_changed_payload(added: bool) -> String {
    serde_json::json!({
        "message": "gamepadChanged",
        "gamepadIndex": 0,
        "wasAdded": added,
    })
    .to_string()
}

pub(crate) fn build_input_metadata_bootstrap_packet() -> Vec<u8> {
    build_input_metadata_packet(
        INPUT_METADATA_SEQ,
        now_ms_f64(),
        INPUT_METADATA_MAX_TOUCHPOINTS,
    )
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

pub(crate) fn build_input_metadata_packet(seq: u32, time: f64, max_touchpoints: u8) -> Vec<u8> {
    let mut packet = Vec::with_capacity(15);
    packet.extend_from_slice(&8u16.to_le_bytes());
    packet.extend_from_slice(&seq.to_le_bytes());
    packet.extend_from_slice(&time.to_le_bytes());
    packet.push(max_touchpoints);
    packet
}
