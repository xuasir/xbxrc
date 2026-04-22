use std::sync::{Arc, Mutex};

use ohmygamepad_protocol::{LogicalPadId, LogicalPadSnapshotDto};
use xbxengine_protocol::XbxEngineInputEventDto;

use crate::XbxEngineMediaRuntimeStats;

// 只保留当前主线仍然消费的最小输入状态。
#[derive(Default)]
pub(crate) struct XbxDataChannelState {
    pub(crate) keyboard_pointer_enabled: bool,
    pub(crate) pending_input_events: Vec<XbxEngineInputEventDto>,
}

// 输入协议包里只保留当前仍在主线使用的几种帧结构。
#[derive(Clone, Copy)]
pub(crate) struct PointerPacketEvent {
    tilt_x: u16,
    tilt_y: u16,
    pressure: u8,
    twist: u16,
    x: u32,
    y: u32,
    event_kind: u8,
}

#[derive(Clone, Copy)]
pub(crate) struct MousePacketFrame {
    x: u32,
    y: u32,
    wheel_x: u32,
    wheel_y: u32,
    buttons: u8,
    relative: u8,
}

#[derive(Clone, Copy)]
pub(crate) struct KeyboardPacketFrame {
    pressed: bool,
    key_code: u8,
}

#[derive(Clone, Copy)]
pub(crate) struct VideoMetadataFrame {
    server_data_key: u32,
    first_frame_packet_arrival_time_ms: u32,
    frame_submitted_time_ms: u32,
    frame_decoded_time_ms: u32,
    frame_rendered_time_ms: u32,
}

pub(crate) const STREAM_INPUT_IDLE_GAMEPAD_KEEPALIVE_MS: u64 = 250;

const MAX_PENDING_INPUT_EVENTS: usize = 64;
const REPORT_TYPE_METADATA: u16 = 1;
const REPORT_TYPE_GAMEPAD: u16 = 2;
const REPORT_TYPE_POINTER: u16 = 4;
const REPORT_TYPE_MOUSE: u16 = 32;
const REPORT_TYPE_KEYBOARD: u16 = 64;

pub(crate) fn set_keyboard_pointer_enabled(
    runtime_state: &Arc<Mutex<XbxDataChannelState>>,
    enabled: bool,
) {
    if let Ok(mut state) = runtime_state.lock() {
        state.keyboard_pointer_enabled = enabled;
        if !enabled {
            state.pending_input_events.clear();
        }
    }
}

pub(crate) fn queue_keyboard_pointer_input(
    runtime_state: &Arc<Mutex<XbxDataChannelState>>,
    event: XbxEngineInputEventDto,
) {
    let Ok(mut state) = runtime_state.lock() else {
        return;
    };
    if !state.keyboard_pointer_enabled {
        return;
    }
    if state.pending_input_events.len() >= MAX_PENDING_INPUT_EVENTS {
        let overflow = state.pending_input_events.len() + 1 - MAX_PENDING_INPUT_EVENTS;
        state.pending_input_events.drain(0..overflow);
    }
    state.pending_input_events.push(event);
}

pub(crate) fn build_input_stream_packet(
    seq: u32,
    time: f64,
    metadata: Option<&VideoMetadataFrame>,
    frames: &[LogicalPadSnapshotDto],
    pointer_events: &[PointerPacketEvent],
    mouse_frames: &[MousePacketFrame],
    keyboard_frames: &[KeyboardPacketFrame],
) -> Vec<u8> {
    let mut report_type = 0u16;
    let mut total_size = 14usize;
    if metadata.is_some() {
        report_type |= REPORT_TYPE_METADATA;
        total_size += 1 + 28;
    }
    if !frames.is_empty() {
        report_type |= REPORT_TYPE_GAMEPAD;
        total_size += 1 + (22 * frames.len());
    }
    if !pointer_events.is_empty() {
        report_type |= REPORT_TYPE_POINTER;
        total_size += 2 + (20 * pointer_events.len());
    }
    if !mouse_frames.is_empty() {
        report_type |= REPORT_TYPE_MOUSE;
        total_size += 1 + (18 * mouse_frames.len());
    }
    if !keyboard_frames.is_empty() {
        report_type |= REPORT_TYPE_KEYBOARD;
        total_size += 1 + (3 * keyboard_frames.len());
    }

    let mut packet = Vec::with_capacity(total_size);
    packet.extend_from_slice(&report_type.to_le_bytes());
    packet.extend_from_slice(&seq.to_le_bytes());
    packet.extend_from_slice(&time.to_le_bytes());

    if let Some(metadata) = metadata {
        packet.push(1);
        packet.extend_from_slice(&metadata.server_data_key.to_le_bytes());
        packet.extend_from_slice(&metadata.first_frame_packet_arrival_time_ms.to_le_bytes());
        packet.extend_from_slice(&metadata.frame_submitted_time_ms.to_le_bytes());
        packet.extend_from_slice(&metadata.frame_decoded_time_ms.to_le_bytes());
        packet.extend_from_slice(&metadata.frame_rendered_time_ms.to_le_bytes());
        let now_u32 = time.max(0.0).round().clamp(0.0, u32::MAX as f64) as u32;
        packet.extend_from_slice(&now_u32.to_le_bytes());
        packet.extend_from_slice(&now_u32.to_le_bytes());
    }

    if !frames.is_empty() {
        packet.push(frames.len().min(u8::MAX as usize) as u8);
    }

    for frame in frames {
        packet.push(logical_pad_index(&frame.slot));
        packet.extend_from_slice(&gamepad_button_mask(&frame.state).to_le_bytes());
        packet.extend_from_slice(&normalize_axis(frame.state.left_stick.x).to_le_bytes());
        packet.extend_from_slice(&normalize_axis(frame.state.left_stick.y).to_le_bytes());
        packet.extend_from_slice(&normalize_axis(frame.state.right_stick.x).to_le_bytes());
        packet.extend_from_slice(&normalize_axis(frame.state.right_stick.y).to_le_bytes());
        packet.extend_from_slice(
            &normalize_trigger(frame.state.buttons.l2.max(frame.state.left_trigger)).to_le_bytes(),
        );
        packet.extend_from_slice(
            &normalize_trigger(frame.state.buttons.r2.max(frame.state.right_trigger)).to_le_bytes(),
        );
        packet.extend_from_slice(&0u32.to_le_bytes());
        packet.extend_from_slice(&0u32.to_be_bytes());
    }

    if !pointer_events.is_empty() {
        packet.push(1);
        packet.push(pointer_events.len().min(u8::MAX as usize) as u8);
        for pointer_event in pointer_events {
            packet.extend_from_slice(&pointer_event.tilt_x.to_le_bytes());
            packet.extend_from_slice(&pointer_event.tilt_y.to_le_bytes());
            packet.push(pointer_event.pressure);
            packet.extend_from_slice(&pointer_event.twist.to_le_bytes());
            packet.extend_from_slice(&0u32.to_le_bytes());
            packet.extend_from_slice(&pointer_event.x.to_le_bytes());
            packet.extend_from_slice(&pointer_event.y.to_le_bytes());
            packet.push(pointer_event.event_kind);
        }
    }

    if !mouse_frames.is_empty() {
        packet.push(mouse_frames.len().min(u8::MAX as usize) as u8);
        for mouse_frame in mouse_frames {
            packet.extend_from_slice(&mouse_frame.x.to_le_bytes());
            packet.extend_from_slice(&mouse_frame.y.to_le_bytes());
            packet.extend_from_slice(&mouse_frame.wheel_x.to_le_bytes());
            packet.extend_from_slice(&mouse_frame.wheel_y.to_le_bytes());
            packet.push(mouse_frame.buttons);
            packet.push(mouse_frame.relative);
        }
    }

    if !keyboard_frames.is_empty() {
        packet.push(keyboard_frames.len().min(u8::MAX as usize) as u8);
        for keyboard_frame in keyboard_frames {
            packet.push(2);
            packet.push(u8::from(keyboard_frame.pressed));
            packet.push(keyboard_frame.key_code);
        }
    }

    packet
}

pub(crate) fn drain_pending_input_frames(
    runtime_state: &Arc<Mutex<XbxDataChannelState>>,
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
) -> (
    Vec<PointerPacketEvent>,
    Vec<MousePacketFrame>,
    Vec<KeyboardPacketFrame>,
) {
    let mut pending_events = Vec::new();
    if let Ok(mut state) = runtime_state.lock() {
        if !state.pending_input_events.is_empty() {
            pending_events = std::mem::take(&mut state.pending_input_events);
        }
    }
    if pending_events.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }

    let (target_width, target_height) = runtime_stats
        .lock()
        .ok()
        .and_then(|stats| {
            stats
                .latest_video_stream_width
                .zip(stats.latest_video_stream_height)
        })
        .unwrap_or((1920, 1080));
    let mut pointer_events = Vec::new();
    let mut mouse_frames = Vec::new();
    let mut keyboard_frames = Vec::new();

    for event in pending_events {
        match event {
            XbxEngineInputEventDto::Pointer {
                event,
                pointer_type,
                x,
                y,
                delta_x,
                delta_y,
                button,
                ..
            } => {
                if pointer_type.eq_ignore_ascii_case("mouse") {
                    mouse_frames.push(build_mouse_frame(
                        &event,
                        x,
                        y,
                        delta_x.unwrap_or_default(),
                        delta_y.unwrap_or_default(),
                        button,
                    ));
                } else {
                    pointer_events.push(build_pointer_event(
                        &event,
                        x,
                        y,
                        target_width,
                        target_height,
                    ));
                }
            }
            XbxEngineInputEventDto::Keyboard {
                event, code, key, ..
            } => {
                keyboard_frames.push(KeyboardPacketFrame {
                    pressed: event.eq_ignore_ascii_case("down"),
                    key_code: keyboard_key_code(&code, &key),
                });
            }
        }
    }

    (pointer_events, mouse_frames, keyboard_frames)
}

pub(crate) fn build_metadata_frame(
    stats: &XbxEngineMediaRuntimeStats,
    last_metadata_frame_seq: &mut u64,
) -> Option<VideoMetadataFrame> {
    let frame = stats.latest_video_frame.as_ref()?;
    if frame.frame_seq <= *last_metadata_frame_seq {
        return None;
    }
    let packet_arrival = stats.latest_video_packet_arrival_time_ms?;
    *last_metadata_frame_seq = frame.frame_seq;
    let frame_time = frame.rendered_at_ms;
    Some(VideoMetadataFrame {
        server_data_key: frame.frame_seq.min(u32::MAX as u64) as u32,
        first_frame_packet_arrival_time_ms: clamp_u32_ms(packet_arrival),
        frame_submitted_time_ms: clamp_u32_ms(packet_arrival),
        frame_decoded_time_ms: clamp_u32_ms(
            stats.latest_video_decode_ok_time_ms.unwrap_or(frame_time),
        ),
        frame_rendered_time_ms: clamp_u32_ms(frame_time),
    })
}

fn build_pointer_event(
    event: &str,
    x: f64,
    y: f64,
    target_width: u32,
    target_height: u32,
) -> PointerPacketEvent {
    let is_release = event.eq_ignore_ascii_case("up") || event.eq_ignore_ascii_case("pointerup");
    let event_kind =
        if event.eq_ignore_ascii_case("down") || event.eq_ignore_ascii_case("pointerdown") {
            1
        } else if is_release {
            2
        } else {
            3
        };
    let tilt_x = if is_release {
        0
    } else {
        (0.065_757_5 * target_height as f64).round() as u16
    };
    let tilt_y = if is_release {
        0
    } else {
        (0.065_757_5 * target_width as f64).round() as u16
    };

    PointerPacketEvent {
        tilt_x,
        tilt_y,
        pressure: if is_release { 0 } else { u8::MAX },
        twist: 0,
        x: if is_release {
            0
        } else {
            clamp_u32_coord(x, target_width)
        },
        y: if is_release {
            0
        } else {
            clamp_u32_coord(y, target_height)
        },
        event_kind,
    }
}

fn build_mouse_frame(
    event: &str,
    x: f64,
    y: f64,
    delta_x: f64,
    delta_y: f64,
    button: Option<u8>,
) -> MousePacketFrame {
    MousePacketFrame {
        x: clamp_u32_coord(x, u32::MAX),
        y: clamp_u32_coord(y, u32::MAX),
        wheel_x: clamp_i32_coord(delta_x),
        wheel_y: clamp_i32_coord(delta_y),
        buttons: if event.eq_ignore_ascii_case("wheel") {
            0
        } else {
            map_mouse_button_mask(button)
        },
        relative: 0,
    }
}

fn map_mouse_button_mask(button: Option<u8>) -> u8 {
    match button.unwrap_or_default() {
        0 => 1,
        1 => 2,
        2 => 4,
        _ => 0,
    }
}

fn clamp_u32_coord(value: f64, max: u32) -> u32 {
    value.max(0.0).round().clamp(0.0, max as f64) as u32
}

fn clamp_i32_coord(value: f64) -> u32 {
    (value.round() as i32).max(0) as u32
}

fn keyboard_key_code(code: &str, key: &str) -> u8 {
    if let Some(digit) = code
        .strip_prefix("Digit")
        .and_then(|value| value.parse::<u8>().ok())
    {
        return 0x30u8.saturating_add(digit.min(9));
    }
    if let Some(letter) = code
        .strip_prefix("Key")
        .and_then(|value| value.as_bytes().first())
    {
        return letter.to_ascii_uppercase();
    }
    match code {
        "ArrowUp" => 0x26,
        "ArrowDown" => 0x28,
        "ArrowLeft" => 0x25,
        "ArrowRight" => 0x27,
        "Enter" | "NumpadEnter" => 0x0D,
        "Escape" => 0x1B,
        "Space" => 0x20,
        "Tab" => 0x09,
        "Backspace" => 0x08,
        "Delete" => 0x2E,
        "Insert" => 0x2D,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "ShiftLeft" | "ShiftRight" => 0x10,
        "ControlLeft" | "ControlRight" => 0x11,
        "AltLeft" | "AltRight" => 0x12,
        "MetaLeft" | "MetaRight" => 0x5B,
        _ => match key {
            " " => 0x20,
            "Enter" => 0x0D,
            "Escape" => 0x1B,
            _ => key
                .chars()
                .next()
                .map(|character| character.to_ascii_uppercase() as u8)
                .unwrap_or_default(),
        },
    }
}

fn logical_pad_index(pad_id: &LogicalPadId) -> u8 {
    match pad_id {
        LogicalPadId::Pad0 => 0,
        LogicalPadId::Pad1 => 1,
        LogicalPadId::Pad2 => 2,
        LogicalPadId::Pad3 => 3,
    }
}

fn gamepad_button_mask(state: &ohmygamepad_protocol::LogicalPadStateDto) -> u16 {
    let buttons = &state.buttons;
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
    let normalized = (value * 32767.0).round();
    normalized.clamp(-32767.0, 32767.0) as i16
}

fn normalize_trigger(value: f32) -> u16 {
    if value <= 0.0 {
        return 0;
    }
    (value * 65535.0).round().clamp(0.0, 65535.0) as u16
}

fn clamp_u32_ms(value: f64) -> u32 {
    value.max(0.0).round().clamp(0.0, u32::MAX as f64) as u32
}
