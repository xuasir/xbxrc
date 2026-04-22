use ohmygamepad_protocol::{
    LogicalPadId, OhMyGamepadRumbleEffectDto, OhMyGamepadRumbleRequestDto,
    OhMyGamepadRumbleTargetDto,
};

const GAMEPAD_RUMBLE_REPORT_TYPE: u16 = 128;
const GAMEPAD_RUMBLE_MESSAGE_TYPE_SIZE: usize = 1;
const GAMEPAD_RUMBLE_MESSAGE_TYPE_SIZE_V8: usize = 2;
const GAMEPAD_RUMBLE_KIND_FOUR_MOTOR: u8 = 0;
const GAMEPAD_RUMBLE_KIND_SIZE: usize = 1;
const GAMEPAD_RUMBLE_PACKET_SIZE_V8: usize = 13;
const GAMEPAD_RUMBLE_HEADER_SIZE_LEGACY: usize = 2;
const GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY: usize = 12;

pub(crate) fn parse_rumble_requests(payload: &[u8]) -> Vec<OhMyGamepadRumbleRequestDto> {
    if let Some(requests) = parse_better_xcloud_packet(payload) {
        return requests;
    }

    parse_legacy_packet(payload).unwrap_or_default()
}

fn parse_better_xcloud_packet(payload: &[u8]) -> Option<Vec<OhMyGamepadRumbleRequestDto>> {
    if payload.len() < GAMEPAD_RUMBLE_PACKET_SIZE_V8 {
        return None;
    }

    let mut offset = 0usize;
    let mut message_type = u16::from(payload[0]);
    let mut message_type_size = GAMEPAD_RUMBLE_MESSAGE_TYPE_SIZE;

    let v8_message_type = u16::from_le_bytes([payload[0], payload[1]]);
    if (v8_message_type & GAMEPAD_RUMBLE_REPORT_TYPE) != 0 {
        message_type = v8_message_type;
        message_type_size = GAMEPAD_RUMBLE_MESSAGE_TYPE_SIZE_V8;
    }

    if (message_type & GAMEPAD_RUMBLE_REPORT_TYPE) == 0 {
        return None;
    }

    offset += message_type_size;
    if offset + GAMEPAD_RUMBLE_KIND_SIZE > payload.len() {
        return None;
    }

    let vibration_type = payload[offset];
    offset += GAMEPAD_RUMBLE_KIND_SIZE;
    if vibration_type != GAMEPAD_RUMBLE_KIND_FOUR_MOTOR {
        return None;
    }

    if offset + 7 > payload.len() {
        return None;
    }

    let target = logical_pad_target_from_index(payload[offset]);
    let effect = OhMyGamepadRumbleEffectDto {
        strong_magnitude: normalize_percent_motor(payload[offset + 1]),
        weak_magnitude: normalize_percent_motor(payload[offset + 2]),
        left_trigger: normalize_percent_motor(payload[offset + 3]),
        right_trigger: normalize_percent_motor(payload[offset + 4]),
        duration_ms: u16::from_le_bytes([payload[offset + 5], payload[offset + 6]]),
        start_delay_ms: 0,
        repeat: 0,
    };

    Some(vec![OhMyGamepadRumbleRequestDto { target, effect }])
}

fn parse_legacy_packet(payload: &[u8]) -> Option<Vec<OhMyGamepadRumbleRequestDto>> {
    if payload.len() < GAMEPAD_RUMBLE_HEADER_SIZE_LEGACY + GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY {
        return None;
    }

    if payload.first().copied()? != GAMEPAD_RUMBLE_REPORT_TYPE as u8 {
        return None;
    }

    let mut requests = Vec::new();
    let mut offset = GAMEPAD_RUMBLE_HEADER_SIZE_LEGACY;

    while offset + GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY <= payload.len() {
        if offset + 11 >= payload.len() {
            break;
        }

        let target = logical_pad_target_from_index(payload[offset + 1]);
        let effect = OhMyGamepadRumbleEffectDto {
            left_trigger: normalize_unit_motor(
                u16::from_le_bytes([payload[offset + 2], payload[offset + 3]]) as f32 / 1023.0,
            ),
            right_trigger: normalize_unit_motor(
                u16::from_le_bytes([payload[offset + 4], payload[offset + 5]]) as f32 / 1023.0,
            ),
            weak_magnitude: normalize_unit_motor(
                u16::from_le_bytes([payload[offset + 6], payload[offset + 7]]) as f32 / 1023.0,
            ),
            strong_magnitude: normalize_unit_motor(
                u16::from_le_bytes([payload[offset + 8], payload[offset + 9]]) as f32 / 1023.0,
            ),
            duration_ms: u16::from_le_bytes([payload[offset + 10], payload[offset + 11]]),
            start_delay_ms: 0,
            repeat: 0,
        };

        requests.push(OhMyGamepadRumbleRequestDto { target, effect });
        offset += GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY;
    }

    Some(requests)
}

fn logical_pad_target_from_index(gamepad_index: u8) -> OhMyGamepadRumbleTargetDto {
    match gamepad_index {
        0 => OhMyGamepadRumbleTargetDto::Slot {
            slot: LogicalPadId::Pad0,
        },
        1 => OhMyGamepadRumbleTargetDto::Slot {
            slot: LogicalPadId::Pad1,
        },
        2 => OhMyGamepadRumbleTargetDto::Slot {
            slot: LogicalPadId::Pad2,
        },
        3 => OhMyGamepadRumbleTargetDto::Slot {
            slot: LogicalPadId::Pad3,
        },
        _ => OhMyGamepadRumbleTargetDto::Auto,
    }
}

fn normalize_percent_motor(value: u8) -> f32 {
    normalize_unit_motor((value.clamp(0, 100) as f32) / 100.0)
}

fn normalize_unit_motor(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::parse_rumble_requests;
    use ohmygamepad_protocol::{LogicalPadId, OhMyGamepadRumbleTargetDto};

    #[test]
    fn parses_better_xcloud_rumble_packet() {
        let packet = vec![128, 0, 0, 0, 12, 5, 3, 2, 64, 0, 0, 0, 0];

        let requests = parse_rumble_requests(&packet);
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].target,
            OhMyGamepadRumbleTargetDto::Slot {
                slot: LogicalPadId::Pad0,
            }
        );
        assert_eq!(requests[0].effect.duration_ms, 64);
    }

    #[test]
    fn ignores_short_or_non_rumble_packets() {
        assert!(parse_rumble_requests(&[1, 2, 3]).is_empty());
        assert!(parse_rumble_requests(&[127, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_empty());
    }

    #[test]
    fn parses_legacy_rumble_packet() {
        let packet = vec![128, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 10, 0];

        let requests = parse_rumble_requests(&packet);
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].target,
            OhMyGamepadRumbleTargetDto::Slot {
                slot: LogicalPadId::Pad1,
            }
        );
        assert_eq!(requests[0].effect.duration_ms, 10);
    }

    #[test]
    fn preserves_zeroed_stop_packet() {
        let packet = vec![128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

        let requests = parse_rumble_requests(&packet);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].effect.duration_ms, 0);
        assert_eq!(requests[0].effect.left_trigger, 0.0);
        assert_eq!(requests[0].effect.right_trigger, 0.0);
        assert_eq!(requests[0].effect.weak_magnitude, 0.0);
        assert_eq!(requests[0].effect.strong_magnitude, 0.0);
    }
}
