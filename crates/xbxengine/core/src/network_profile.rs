use ohmygamepad_protocol::{
    LogicalPadId, LogicalPadSnapshotDto, OhMyGamepadRumbleEffectDto, OhMyGamepadRumbleRequestDto,
    OhMyGamepadRumbleTargetDto,
};
use serde_json::{json, Value};

use crate::webrtc_rs_negotiation_profile::current_webrtc_rs_negotiation_profile;

pub struct StreamDataChannelProfile {
    pub name: &'static str,
    pub protocol: &'static str,
    pub ordered: bool,
}

// 这组常量需要与 renderer/browser player 严格一致，避免 Rust/Web 网络层继续漂移。
pub const STREAM_DATA_CHANNEL_PROFILES: [StreamDataChannelProfile; 4] = [
    StreamDataChannelProfile {
        name: "input",
        protocol: "1.0",
        ordered: true,
    },
    StreamDataChannelProfile {
        name: "control",
        protocol: "controlV1",
        ordered: true,
    },
    StreamDataChannelProfile {
        name: "chat",
        protocol: "chatV1",
        ordered: true,
    },
    StreamDataChannelProfile {
        name: "message",
        protocol: "messageV1",
        ordered: true,
    },
];

pub const STREAM_MESSAGE_HANDSHAKE_ID: &str = "f9c5f412-0e69-4ede-8e62-92c7f5358c56";
pub const STREAM_MESSAGE_TRANSACTION_ID: &str = "41f93d5a-900f-4d33-b7a1-2d4ca6747072";
pub const STREAM_CLIENT_APP_INSTALL_ID: &str = "c11ddb2e-c7e3-4f02-a62b-fd5448e0b851";
pub const STREAM_MESSAGE_VERSION: &str = "messageV1";
pub const STREAM_CONTROL_ACCESS_KEY: &str = "4BDB3609-C1F1-4195-9B37-FEFF45DA8B8E";
pub const STREAM_CONTROL_DEFAULT_GAMEPAD_INDEX: u8 = 0;
pub const STREAM_INPUT_INITIAL_MAX_TOUCHPOINTS: u8 = 2;
pub const STREAM_CONTROL_GAMEPAD_ADDED_DELAY_MS: u64 = 500;
pub const STREAM_CONTROL_KEYFRAME_INTERVAL_MS: u64 = 5_000;
pub const STREAM_INPUT_POLL_INTERVAL_MS: u64 = 8;
pub const STREAM_INPUT_MAX_BUFFERED_AMOUNT_BYTES: usize = 256 * 1024;

const INPUT_REPORT_TYPE_GAMEPAD: u16 = 2;
const INPUT_REPORT_TYPE_VIBRATION: u8 = 128;
const INPUT_GAMEPAD_FRAME_SIZE: usize = 22;
const INPUT_GAMEPAD_PACKET_HEADER_SIZE: usize = 14;
const INPUT_RUMBLE_HEADER_SIZE: usize = 2;
const INPUT_RUMBLE_PAYLOAD_SIZE: usize = 11;

pub fn build_message_handshake_payload() -> String {
    json!({
        "type": "Handshake",
        "version": STREAM_MESSAGE_VERSION,
        "id": STREAM_MESSAGE_HANDSHAKE_ID,
        "cv": "",
    })
    .to_string()
}

pub fn is_handshake_ack_payload(payload: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return false;
    };
    if value.get("type").and_then(Value::as_str) != Some("HandshakeAck") {
        return false;
    }

    // 线上 ack 字段偶发不完整，id/version 缺失时保持兼容；存在时要求和值匹配。
    let id_matches = value
        .get("id")
        .and_then(Value::as_str)
        .is_none_or(|id| id == STREAM_MESSAGE_HANDSHAKE_ID);
    let version_matches = value
        .get("version")
        .and_then(Value::as_str)
        .is_none_or(|version| version == STREAM_MESSAGE_VERSION);
    id_matches && version_matches
}

fn build_message_payload(target: &str, content: Value) -> String {
    json!({
        "type": "Message",
        "content": content.to_string(),
        "id": STREAM_MESSAGE_TRANSACTION_ID,
        "target": target,
        "cv": "",
    })
    .to_string()
}

pub fn build_post_handshake_message_payloads() -> Vec<String> {
    let profile = current_webrtc_rs_negotiation_profile();
    let viewport_width = profile.width;
    let viewport_height = profile.height;
    vec![
        build_message_payload(
            "/streaming/systemUi/configuration",
            json!({
                "version": [0, 2, 0],
                "systemUis": [10, 19, 31, 27, 32, -41]
            }),
        ),
        build_message_payload(
            "/streaming/properties/clientappinstallidchanged",
            json!({
                "clientAppInstallId": STREAM_CLIENT_APP_INSTALL_ID
            }),
        ),
        build_message_payload(
            "/streaming/characteristics/orientationchanged",
            json!({
                "orientation": 0
            }),
        ),
        build_message_payload(
            "/streaming/characteristics/touchinputenabledchanged",
            json!({
                "touchInputEnabled": false
            }),
        ),
        build_message_payload(
            "/streaming/characteristics/clientdevicecapabilities",
            json!({}),
        ),
        build_message_payload(
            "/streaming/characteristics/dimensionschanged",
            json!({
                "horizontal": viewport_width,
                "vertical": viewport_height,
                "preferredWidth": viewport_width,
                "preferredHeight": viewport_height,
                "safeAreaLeft": 0,
                "safeAreaTop": 0,
                "safeAreaRight": viewport_width,
                "safeAreaBottom": viewport_height,
                "supportsCustomResolution": true
            }),
        ),
    ]
}

pub fn build_control_authorization_payload() -> String {
    json!({
        "message": "authorizationRequest",
        "accessKey": STREAM_CONTROL_ACCESS_KEY,
    })
    .to_string()
}

pub fn build_control_gamepad_changed_payload(was_added: bool) -> String {
    json!({
        "message": "gamepadChanged",
        "gamepadIndex": STREAM_CONTROL_DEFAULT_GAMEPAD_INDEX,
        "wasAdded": was_added,
    })
    .to_string()
}

pub fn build_control_keyframe_request_payload() -> String {
    json!({
        "message": "videoKeyframeRequested",
        "ifrRequested": true,
    })
    .to_string()
}

pub fn build_input_metadata_packet(sequence: u32, timestamp_ms: f64) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(15);
    buffer.extend_from_slice(&(8u16).to_le_bytes());
    buffer.extend_from_slice(&sequence.to_le_bytes());
    buffer.extend_from_slice(&timestamp_ms.to_le_bytes());
    buffer.push(STREAM_INPUT_INITIAL_MAX_TOUCHPOINTS);
    buffer
}

pub fn build_input_gamepad_packet(
    sequence: u32,
    timestamp_ms: f64,
    frames: &[LogicalPadSnapshotDto],
) -> Vec<u8> {
    let frame_count = frames.len().min(u8::MAX as usize);
    let mut buffer = Vec::with_capacity(
        INPUT_GAMEPAD_PACKET_HEADER_SIZE + 1 + frame_count * INPUT_GAMEPAD_FRAME_SIZE,
    );
    buffer.extend_from_slice(&INPUT_REPORT_TYPE_GAMEPAD.to_le_bytes());
    buffer.extend_from_slice(&sequence.to_le_bytes());
    buffer.extend_from_slice(&timestamp_ms.to_le_bytes());
    buffer.push(frame_count as u8);

    for frame in frames.iter().take(frame_count) {
        buffer.push(logical_pad_index(frame.pad_id));
        buffer.extend_from_slice(&build_gamepad_button_mask(frame).to_le_bytes());
        buffer.extend_from_slice(&normalize_axis(frame.state.left_stick.x).to_le_bytes());
        buffer.extend_from_slice(&normalize_axis(frame.state.left_stick.y).to_le_bytes());
        buffer.extend_from_slice(&normalize_axis(frame.state.right_stick.x).to_le_bytes());
        buffer.extend_from_slice(&normalize_axis(frame.state.right_stick.y).to_le_bytes());
        buffer.extend_from_slice(
            &normalize_trigger(frame.state.buttons.l2.max(frame.state.left_trigger)).to_le_bytes(),
        );
        buffer.extend_from_slice(
            &normalize_trigger(frame.state.buttons.r2.max(frame.state.right_trigger)).to_le_bytes(),
        );
        buffer.extend_from_slice(&0u32.to_le_bytes());
        buffer.extend_from_slice(&0u32.to_be_bytes());
    }

    buffer
}

pub fn parse_input_rumble_packet(payload: &[u8]) -> Option<OhMyGamepadRumbleRequestDto> {
    if payload.len() < INPUT_RUMBLE_HEADER_SIZE + INPUT_RUMBLE_PAYLOAD_SIZE {
        return None;
    }
    if payload[0] != INPUT_REPORT_TYPE_VIBRATION {
        return None;
    }

    let gamepad_index = payload[3];
    let target = OhMyGamepadRumbleTargetDto::LogicalPad {
        pad_id: logical_pad_id_from_index(gamepad_index)?,
    };

    let left_motor_percent = payload[4] as f32 / 100.0;
    let right_motor_percent = payload[5] as f32 / 100.0;
    let left_trigger_percent = payload[6] as f32 / 100.0;
    let right_trigger_percent = payload[7] as f32 / 100.0;
    let raw_duration_ms = u16::from_le_bytes([payload[8], payload[9]]);
    let repeat = payload[12];

    Some(OhMyGamepadRumbleRequestDto {
        target,
        effect: OhMyGamepadRumbleEffectDto {
            start_delay_ms: 0,
            // 先与现有 web 路径保持一致，避免 Rust/Web 体感进一步漂移。
            duration_ms: (raw_duration_ms / 10).max(1),
            strong_magnitude: quantize_magnitude(left_motor_percent),
            weak_magnitude: quantize_magnitude(right_motor_percent),
            left_trigger: quantize_magnitude(left_trigger_percent),
            right_trigger: quantize_magnitude(right_trigger_percent),
            repeat,
        },
    })
}

fn logical_pad_index(pad_id: LogicalPadId) -> u8 {
    match pad_id {
        LogicalPadId::Pad0 => 0,
        LogicalPadId::Pad1 => 1,
        LogicalPadId::Pad2 => 2,
        LogicalPadId::Pad3 => 3,
    }
}

fn logical_pad_id_from_index(gamepad_index: u8) -> Option<LogicalPadId> {
    match gamepad_index {
        0 => Some(LogicalPadId::Pad0),
        1 => Some(LogicalPadId::Pad1),
        2 => Some(LogicalPadId::Pad2),
        3 => Some(LogicalPadId::Pad3),
        _ => None,
    }
}

fn build_gamepad_button_mask(frame: &LogicalPadSnapshotDto) -> u16 {
    let buttons = frame.state.buttons;
    let mut mask = 0u16;
    if buttons.home > 0.0 {
        mask |= 2;
    }
    if buttons.menu > 0.0 {
        mask |= 4;
    }
    if buttons.view > 0.0 {
        mask |= 8;
    }
    if buttons.south > 0.0 {
        mask |= 16;
    }
    if buttons.east > 0.0 {
        mask |= 32;
    }
    if buttons.west > 0.0 {
        mask |= 64;
    }
    if buttons.north > 0.0 {
        mask |= 128;
    }
    if buttons.dpad_up > 0.0 {
        mask |= 256;
    }
    if buttons.dpad_down > 0.0 {
        mask |= 512;
    }
    if buttons.dpad_left > 0.0 {
        mask |= 1024;
    }
    if buttons.dpad_right > 0.0 {
        mask |= 2048;
    }
    if buttons.l1 > 0.0 {
        mask |= 4096;
    }
    if buttons.r1 > 0.0 {
        mask |= 8192;
    }
    if buttons.l3 > 0.0 {
        mask |= 16384;
    }
    if buttons.r3 > 0.0 {
        mask |= 32768;
    }
    mask
}

fn normalize_axis(value: f32) -> i16 {
    let max = 32_767_f32;
    let min = -32_767_f32;
    value.mul_add(max, 0.0).clamp(min, max).round() as i16
}

fn normalize_trigger(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65_535.0)
        .round()
        .clamp(0.0, 65_535.0) as u16
}

fn quantize_magnitude(value: f32) -> f32 {
    let normalized = value.clamp(0.0, 1.0);
    if normalized < 0.05 {
        return 0.0;
    }
    (normalized * 16.0).round() / 16.0
}

#[cfg(test)]
mod tests {
    use ohmygamepad_protocol::{LogicalButtonsStateDto, LogicalPadSnapshotDto, LogicalPadStateDto};

    use super::{build_input_gamepad_packet, parse_input_rumble_packet};

    #[test]
    fn gamepad_packet_matches_legacy_layout() {
        let packet = build_input_gamepad_packet(
            7,
            1234.0,
            &[LogicalPadSnapshotDto {
                state: LogicalPadStateDto {
                    buttons: LogicalButtonsStateDto {
                        south: 1.0,
                        menu: 1.0,
                        ..Default::default()
                    },
                    left_stick: ohmygamepad_protocol::LogicalStickDto { x: 0.5, y: -0.5 },
                    right_stick: ohmygamepad_protocol::LogicalStickDto { x: 0.25, y: -0.25 },
                    left_trigger: 0.5,
                    right_trigger: 1.0,
                },
                ..Default::default()
            }],
        );

        assert_eq!(u16::from_le_bytes([packet[0], packet[1]]), 2);
        assert_eq!(
            u32::from_le_bytes([packet[2], packet[3], packet[4], packet[5]]),
            7
        );
        assert_eq!(packet[14], 1);
        assert_eq!(packet[15], 0);
        assert_eq!(u16::from_le_bytes([packet[16], packet[17]]), 20);
    }

    #[test]
    fn rumble_packet_parses_to_logical_pad_request() {
        let packet = [128, 0, 0, 1, 100, 50, 25, 75, 200, 0, 10, 0, 2];
        let request = parse_input_rumble_packet(&packet).expect("rumble packet should parse");
        assert_eq!(
            request.target,
            ohmygamepad_protocol::OhMyGamepadRumbleTargetDto::LogicalPad {
                pad_id: ohmygamepad_protocol::LogicalPadId::Pad1
            }
        );
        assert_eq!(request.effect.duration_ms, 20);
        assert_eq!(request.effect.repeat, 2);
        assert_eq!(request.effect.strong_magnitude, 1.0);
    }
}
