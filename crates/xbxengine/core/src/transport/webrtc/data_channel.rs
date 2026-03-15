use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use ohmygamepad_host::GamepadRuntimeHost;
use ohmygamepad_protocol::{
    LogicalPadId, LogicalPadSnapshotDto, OhMyGamepadRouteTargetDto, OhMyGamepadRumbleEffectDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleTargetDto,
};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, Duration};
use webrtc::data_channel::{data_channel_state::RTCDataChannelState, RTCDataChannel};
use xbxengine_protocol::XbxEngineInputEventDto;

use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError};

#[derive(Default)]
pub(crate) struct XbxDataChannelState {
    message_handshake_acked: bool,
    control_started: bool,
    control_bootstrapped_after_handshake: bool,
    chat_open: bool,
    control_channel: Option<Arc<RTCDataChannel>>,
    pending_keyframe_request: bool,
    pending_decoder_reset: bool,
    keyboard_pointer_enabled: bool,
    input_metadata_sent: bool,
    input_metadata_bootstrapped_after_handshake: bool,
    input_stream_loop_started: bool,
    pending_input_events: Vec<XbxEngineInputEventDto>,
    control_gamepad_added_task: Option<JoinHandle<()>>,
    control_keyframe_prime_task: Option<JoinHandle<()>>,
    input_stream_loop_task: Option<JoinHandle<()>>,
}

const MAX_PENDING_INPUT_EVENTS: usize = 64;

#[derive(Clone, Copy)]
struct PointerPacketEvent {
    tilt_x: u16,
    tilt_y: u16,
    pressure: u8,
    twist: u16,
    x: u32,
    y: u32,
    event_kind: u8,
}

#[derive(Clone, Copy)]
struct MousePacketFrame {
    x: u32,
    y: u32,
    wheel_x: u32,
    wheel_y: u32,
    buttons: u8,
    relative: u8,
}

#[derive(Clone, Copy)]
struct KeyboardPacketFrame {
    pressed: bool,
    key_code: u8,
}

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

pub(crate) fn install_data_channel_contracts(
    data_channels: &BTreeMap<String, Arc<RTCDataChannel>>,
    runtime_state: Arc<Mutex<XbxDataChannelState>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) -> Result<(), XbxEngineRuntimeError> {
    let Some(message_channel) = data_channels.get("message").cloned() else {
        return Err(XbxEngineRuntimeError::new("xbxEngineMessageChannelMissing"));
    };
    let Some(control_channel) = data_channels.get("control").cloned() else {
        return Err(XbxEngineRuntimeError::new("xbxEngineControlChannelMissing"));
    };
    let Some(input_channel) = data_channels.get("input").cloned() else {
        return Err(XbxEngineRuntimeError::new("xbxEngineInputChannelMissing"));
    };
    let chat_channel = data_channels.get("chat").cloned();
    if let Ok(mut state) = runtime_state.lock() {
        state.control_channel = Some(control_channel.clone());
    }

    for (label, channel) in data_channels {
        let label = label.clone();
        let label_for_open = label.clone();
        let runtime_state_for_open = runtime_state.clone();
        let runtime_state_for_close = runtime_state.clone();
        let message_channel_for_open = message_channel.clone();
        let control_channel_for_open = control_channel.clone();
        let input_channel_for_open = input_channel.clone();
        let chat_channel_for_open = chat_channel.clone();
        let runtime_stats_for_open = runtime_stats.clone();
        channel.on_open(Box::new(move || {
            let label = label_for_open.clone();
            let runtime_state = runtime_state_for_open.clone();
            let message_channel = message_channel_for_open.clone();
            let control_channel = control_channel_for_open.clone();
            let input_channel = input_channel_for_open.clone();
            let chat_channel = chat_channel_for_open.clone();
            let runtime_stats = runtime_stats_for_open.clone();
            Box::pin(async move {
                crate::xbx_log_debug!("[xbxengine][webrtc-rs] data channel open label={label}");
                if label == "message" {
                    let _ = message_channel
                        .send_text(build_message_handshake_payload())
                        .await;
                } else if label == "chat" {
                    if let Ok(mut state) = runtime_state.lock() {
                        state.chat_open = true;
                    }
                    let _ = chat_channel;
                } else if label == "control" || label == "input" {
                    bootstrap_post_handshake_channels(
                        runtime_state,
                        message_channel,
                        control_channel,
                        input_channel,
                        runtime_stats,
                    )
                    .await;
                }
            })
        }));
        let label_for_close = label.clone();
        channel.on_close(Box::new(move || {
            let label = label_for_close.clone();
            let runtime_state = runtime_state_for_close.clone();
            Box::pin(async move {
                crate::xbx_log_debug!("[xbxengine][webrtc-rs] data channel close label={label}");
                reset_state_on_channel_close(&runtime_state, &label);
            })
        }));

        if label != "message" {
            channel.on_message(Box::new(move |message| {
                let label = label.clone();
                Box::pin(async move {
                    if label == "input" && !message.is_string {
                        handle_input_channel_binary_message(&message.data).await;
                        return;
                    }

                    if message.is_string {
                        if let Ok(payload) = String::from_utf8(message.data.to_vec()) {
                            crate::xbx_log_debug!(
                                "[xbxengine][webrtc-rs] data channel message label={} len={}",
                                label,
                                payload.len()
                            );
                        }
                    }
                })
            }));
        }
    }

    let runtime_state = runtime_state.clone();
    let message_channel_for_message = message_channel.clone();
    let control_channel_for_message = control_channel.clone();
    let input_channel_for_message = input_channel.clone();
    let runtime_stats_for_message = runtime_stats.clone();
    message_channel.on_message(Box::new(move |message| {
        let runtime_state = runtime_state.clone();
        let message_channel = message_channel_for_message.clone();
        let control_channel = control_channel_for_message.clone();
        let input_channel = input_channel_for_message.clone();
        let runtime_stats = runtime_stats_for_message.clone();
        Box::pin(async move {
            if !message.is_string {
                return;
            }
            let Ok(payload) = String::from_utf8(message.data.to_vec()) else {
                return;
            };
            crate::xbx_log_debug!(
                "[xbxengine][webrtc-rs] data channel message label=message len={}",
                payload.len()
            );
            if !is_handshake_ack_payload(&payload) {
                return;
            }

            let mut should_send_post_handshake_messages = false;
            if let Ok(mut state) = runtime_state.lock() {
                if !state.message_handshake_acked {
                    state.message_handshake_acked = true;
                    should_send_post_handshake_messages = true;
                }
            }

            if should_send_post_handshake_messages {
                for payload in build_post_handshake_message_payloads() {
                    let _ = message_channel.send_text(payload).await;
                }
            }

            bootstrap_post_handshake_channels(
                runtime_state,
                message_channel,
                control_channel,
                input_channel,
                runtime_stats,
            )
            .await;
        })
    }));

    Ok(())
}

pub(crate) async fn request_video_keyframe_on_control_channel(
    control_channel: &Arc<RTCDataChannel>,
) -> Result<(), XbxEngineRuntimeError> {
    if control_channel.ready_state() != RTCDataChannelState::Open {
        return Ok(());
    }

    control_channel
        .send_text(build_control_keyframe_request_payload())
        .await
        .map(|_| ())
        .map_err(|error| {
            XbxEngineRuntimeError::new(format!("sendControlKeyframeRequestFailed:{error}"))
        })
}

pub(crate) async fn request_video_keyframe_from_state(
    runtime_state: &Arc<Mutex<XbxDataChannelState>>,
) -> Result<(), XbxEngineRuntimeError> {
    let control_channel = {
        let Ok(mut state) = runtime_state.lock() else {
            return Ok(());
        };
        match state.control_channel.clone() {
            Some(channel) if channel.ready_state() == RTCDataChannelState::Open => Some(channel),
            _ => {
                // control channel 未恢复时先保留恢复意图，避免重连后长时间卡在 waitKeyframe。
                state.pending_keyframe_request = true;
                crate::xbx_log_warn!(
                    "[xbxengine][webrtc-rs] queue pending keyframe request until control channel is open"
                );
                None
            }
        }
    };
    let Some(control_channel) = control_channel else {
        return Ok(());
    };
    match request_video_keyframe_on_control_channel(&control_channel).await {
        Ok(()) => {
            if let Ok(mut state) = runtime_state.lock() {
                state.pending_keyframe_request = false;
            }
            Ok(())
        }
        Err(error) => {
            if let Ok(mut state) = runtime_state.lock() {
                state.pending_keyframe_request = true;
            }
            Err(error)
        }
    }
}

pub(crate) async fn request_decoder_reset_from_state(
    runtime_state: &Arc<Mutex<XbxDataChannelState>>,
) -> Result<(), XbxEngineRuntimeError> {
    let control_channel = {
        let Ok(mut state) = runtime_state.lock() else {
            return Ok(());
        };
        match state.control_channel.clone() {
            Some(channel) if channel.ready_state() == RTCDataChannelState::Open => Some(channel),
            _ => {
                state.pending_decoder_reset = true;
                crate::xbx_log_warn!(
                    "[xbxengine][webrtc-rs] queue pending decoder reset until control channel is open"
                );
                None
            }
        }
    };
    let Some(control_channel) = control_channel else {
        return Ok(());
    };
    match request_decoder_reset_on_control_channel(&control_channel).await {
        Ok(()) => {
            if let Ok(mut state) = runtime_state.lock() {
                state.pending_decoder_reset = false;
                state.pending_keyframe_request = false;
            }
            Ok(())
        }
        Err(error) => {
            if let Ok(mut state) = runtime_state.lock() {
                state.pending_decoder_reset = true;
            }
            Err(error)
        }
    }
}

pub(crate) async fn request_decoder_reset_on_control_channel(
    control_channel: &Arc<RTCDataChannel>,
) -> Result<(), XbxEngineRuntimeError> {
    if control_channel.ready_state() != RTCDataChannelState::Open {
        return Ok(());
    }

    // 先发送独立 decoder reset 意图；若远端不识别，再由关键帧请求兜底。
    if let Err(error) = control_channel
        .send_text(build_control_decoder_reset_payload())
        .await
    {
        crate::xbx_log_error!("[xbxengine][webrtc-rs] send decoder reset payload failed: {error}");
    }

    request_video_keyframe_on_control_channel(control_channel).await
}

async fn bootstrap_post_handshake_channels(
    runtime_state: Arc<Mutex<XbxDataChannelState>>,
    message_channel: Arc<RTCDataChannel>,
    control_channel: Arc<RTCDataChannel>,
    input_channel: Arc<RTCDataChannel>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) {
    let mut should_bootstrap_control = false;
    let mut should_send_input_metadata = false;
    let mut should_start_input_stream_loop = false;
    let mut message_handshake_acked = false;

    if let Ok(mut state) = runtime_state.lock() {
        message_handshake_acked = state.message_handshake_acked;
        if control_channel.ready_state() == RTCDataChannelState::Open
            && (!state.control_started
                || (message_handshake_acked && !state.control_bootstrapped_after_handshake))
        {
            state.control_started = true;
            if message_handshake_acked {
                state.control_bootstrapped_after_handshake = true;
            }
            should_bootstrap_control = true;
        }
        if input_channel.ready_state() == RTCDataChannelState::Open
            && (!state.input_metadata_sent
                || (message_handshake_acked
                    && !state.input_metadata_bootstrapped_after_handshake))
        {
            state.input_metadata_sent = true;
            if message_handshake_acked {
                state.input_metadata_bootstrapped_after_handshake = true;
            }
            should_send_input_metadata = true;
        }
        if !state.input_stream_loop_started
            && input_channel.ready_state() == RTCDataChannelState::Open
        {
            state.input_stream_loop_started = true;
            should_start_input_stream_loop = true;
        }
    }

    if should_bootstrap_control {
        // control/input 不应被 message ack 拖住；
        // 否则媒体已连上时会出现“画面还在 waitKeyframe、输入也没反应”的假死窗口。
        if !message_handshake_acked {
            crate::xbx_log_warn!(
                "[xbxengine][webrtc-rs] bootstrap control before message handshake ack"
            );
        } else {
            crate::xbx_log_info!(
                "[xbxengine][webrtc-rs] bootstrap control after message handshake ack"
            );
        }
        let _ = control_channel
            .send_text(build_control_authorization_payload())
            .await;
        let _ = control_channel
            .send_text(build_control_gamepad_changed_payload(false))
            .await;
        let _ = control_channel
            .send_text(build_control_keyframe_request_payload())
            .await;
        let delayed_added_task = start_delayed_gamepad_added(control_channel.clone());
        let delayed_keyframe_prime_task =
            start_delayed_keyframe_prime(control_channel.clone());
        if let Ok(mut state) = runtime_state.lock() {
            replace_task_handle(&mut state.control_gamepad_added_task, delayed_added_task);
            replace_task_handle(
                &mut state.control_keyframe_prime_task,
                delayed_keyframe_prime_task,
            );
        }
    }
    flush_pending_recovery_requests(runtime_state.clone(), control_channel.clone()).await;
    if should_send_input_metadata {
        if !message_handshake_acked {
            crate::xbx_log_warn!(
                "[xbxengine][webrtc-rs] bootstrap input metadata before message handshake ack"
            );
        } else {
            crate::xbx_log_info!(
                "[xbxengine][webrtc-rs] bootstrap input metadata after message handshake ack"
            );
        }
        let _ = input_channel
            .send(&Bytes::from(build_input_metadata_packet(
                0,
                now_ms_f64(),
                STREAM_INPUT_INITIAL_MAX_TOUCHPOINTS,
            )))
            .await;
    }
    if should_start_input_stream_loop {
        let input_stream_loop_task =
            start_input_stream_loop(input_channel, runtime_state.clone(), runtime_stats);
        if let Ok(mut state) = runtime_state.lock() {
            replace_task_handle(&mut state.input_stream_loop_task, input_stream_loop_task);
        }
    }

    let _ = message_channel;
}

async fn flush_pending_recovery_requests(
    runtime_state: Arc<Mutex<XbxDataChannelState>>,
    control_channel: Arc<RTCDataChannel>,
) {
    if control_channel.ready_state() != RTCDataChannelState::Open {
        return;
    }

    let (flush_decoder_reset, flush_keyframe) = {
        let Ok(state) = runtime_state.lock() else {
            return;
        };
        (
            state.pending_decoder_reset,
            state.pending_keyframe_request && !state.pending_decoder_reset,
        )
    };

    if flush_decoder_reset {
        match request_decoder_reset_on_control_channel(&control_channel).await {
            Ok(()) => {
                if let Ok(mut state) = runtime_state.lock() {
                    state.pending_decoder_reset = false;
                    state.pending_keyframe_request = false;
                }
                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] flushed pending decoder reset after control channel recovery"
                );
            }
            Err(error) => {
                crate::xbx_log_warn!(
                    "[xbxengine][webrtc-rs] flush pending decoder reset failed: {error}"
                );
            }
        }
        return;
    }

    if flush_keyframe {
        match request_video_keyframe_on_control_channel(&control_channel).await {
            Ok(()) => {
                if let Ok(mut state) = runtime_state.lock() {
                    state.pending_keyframe_request = false;
                }
                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] flushed pending keyframe request after control channel recovery"
                );
            }
            Err(error) => {
                crate::xbx_log_warn!(
                    "[xbxengine][webrtc-rs] flush pending keyframe request failed: {error}"
                );
            }
        }
    }
}

fn start_delayed_gamepad_added(control_channel: Arc<RTCDataChannel>) -> JoinHandle<()> {
    tokio::spawn(async move {
        // 与 web player 保持一致：先 removed，再延迟 added，避免会话初期状态抖动。
        sleep(Duration::from_millis(STREAM_CONTROL_GAMEPAD_ADDED_DELAY_MS)).await;
        if control_channel.ready_state() != RTCDataChannelState::Open {
            return;
        }
        let _ = control_channel
            .send_text(build_control_gamepad_changed_payload(true))
            .await;
    })
}

fn start_delayed_keyframe_prime(control_channel: Arc<RTCDataChannel>) -> JoinHandle<()> {
    tokio::spawn(async move {
        // 进入游戏/切场景时，首轮 keyframe 请求可能落在远端尚未完全切入 gameplay 的窗口。
        // 这里补一发短延迟 prime，请求代价很低，但能明显缩短“声音先出来、画面仍卡住”的空窗。
        sleep(Duration::from_millis(STREAM_CONTROL_KEYFRAME_PRIME_DELAY_MS)).await;
        if control_channel.ready_state() != RTCDataChannelState::Open {
            return;
        }
        let _ = control_channel
            .send_text(build_control_keyframe_request_payload())
            .await;
    })
}

fn start_input_stream_loop(
    input_channel: Arc<RTCDataChannel>,
    runtime_state: Arc<Mutex<XbxDataChannelState>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Ok(host) = GamepadRuntimeHost::shared() else {
            return;
        };
        let mut ticker = interval(Duration::from_millis(STREAM_INPUT_POLL_INTERVAL_MS));
        let mut sequence = 1u32;
        let mut last_sample_count = 0usize;
        let mut last_sample_signature = [0u64; 4];
        let mut sample_signature = [0u64; 4];
        let mut frames = Vec::with_capacity(4);
        let mut last_metadata_frame_seq = 0u64;
        let mut last_input_packet_sent_at_ms = now_ms_f64();

        // input loop 拉起后先立刻发一次中性包，避免用户必须按一下手柄远端才开始真正刷新。
        send_idle_gamepad_keepalive(
            &input_channel,
            &host,
            &runtime_state,
            &runtime_stats,
            &mut sequence,
            &mut last_sample_count,
            &mut last_sample_signature,
            &mut sample_signature,
            &mut frames,
            &mut last_metadata_frame_seq,
            &mut last_input_packet_sent_at_ms,
        )
        .await;

        // 与 web input 定时器对齐：后续节奏仍按 polling 周期推进。
        ticker.tick().await;

        loop {
            ticker.tick().await;
            if input_channel.ready_state() != RTCDataChannelState::Open {
                break;
            }

            let Ok(snapshot) = host.snapshot() else {
                continue;
            };
            if !matches!(
                snapshot.route_target,
                OhMyGamepadRouteTargetDto::StreamSession { .. }
            ) {
                continue;
            }
            frames.clear();
            let mut sample_count = 0usize;
            for frame in snapshot.pads.iter().take(4) {
                if sample_count < sample_signature.len() {
                    sample_signature[sample_count] = frame.sample_seq;
                }
                sample_count += 1;
                frames.push(frame.clone());
            }
            let metadata = runtime_stats
                .lock()
                .ok()
                .and_then(|stats| build_metadata_frame(&stats, &mut last_metadata_frame_seq));
            let (pointer_events, mouse_frames, keyboard_frames) =
                drain_pending_input_frames(&runtime_state, runtime_stats.as_ref());
            let gamepad_changed = sample_count > 0
                && !(sample_count == last_sample_count
                    && sample_signature[..sample_count]
                        == last_sample_signature[..last_sample_count]);
            let now_ms = now_ms_f64();
            let should_send_idle_gamepad_keepalive = sample_count > 0
                && !gamepad_changed
                && pointer_events.is_empty()
                && mouse_frames.is_empty()
                && keyboard_frames.is_empty()
                && (now_ms - last_input_packet_sent_at_ms)
                    >= STREAM_INPUT_IDLE_GAMEPAD_KEEPALIVE_MS as f64;
            if metadata.is_none()
                && !gamepad_changed
                && !should_send_idle_gamepad_keepalive
                && pointer_events.is_empty()
                && mouse_frames.is_empty()
                && keyboard_frames.is_empty()
            {
                continue;
            }

            // channel 拥塞时丢旧帧，避免 buffered amount 持续上升导致输入延迟。
            if input_channel.buffered_amount().await
                > STREAM_INPUT_MAX_BUFFERED_AMOUNT_BYTES as usize
            {
                continue;
            }

            let packet = build_input_stream_packet(
                sequence,
                now_ms,
                metadata.as_ref(),
                if gamepad_changed || should_send_idle_gamepad_keepalive {
                    &frames
                } else {
                    &[]
                },
                &pointer_events,
                &mouse_frames,
                &keyboard_frames,
            );
            if input_channel.send(&Bytes::from(packet)).await.is_ok() {
                sequence = sequence.wrapping_add(1);
                last_input_packet_sent_at_ms = now_ms;
                if gamepad_changed || should_send_idle_gamepad_keepalive {
                    last_sample_count = sample_count;
                    last_sample_signature[..sample_count]
                        .copy_from_slice(&sample_signature[..sample_count]);
                }
            }
        }
    })
}

async fn send_idle_gamepad_keepalive(
    input_channel: &Arc<RTCDataChannel>,
    host: &GamepadRuntimeHost,
    runtime_state: &Arc<Mutex<XbxDataChannelState>>,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    sequence: &mut u32,
    last_sample_count: &mut usize,
    last_sample_signature: &mut [u64; 4],
    sample_signature: &mut [u64; 4],
    frames: &mut Vec<LogicalPadSnapshotDto>,
    last_metadata_frame_seq: &mut u64,
    last_input_packet_sent_at_ms: &mut f64,
) {
    if input_channel.ready_state() != RTCDataChannelState::Open {
        return;
    }
    let Ok(snapshot) = host.snapshot() else {
        return;
    };
    if !matches!(
        snapshot.route_target,
        OhMyGamepadRouteTargetDto::StreamSession { .. }
    ) {
        return;
    }

    frames.clear();
    let mut sample_count = 0usize;
    for frame in snapshot.pads.iter().take(4) {
        if sample_count < sample_signature.len() {
            sample_signature[sample_count] = frame.sample_seq;
        }
        sample_count += 1;
        frames.push(frame.clone());
    }
    if sample_count == 0 {
        return;
    }

    let metadata = runtime_stats
        .lock()
        .ok()
        .and_then(|stats| build_metadata_frame(&stats, last_metadata_frame_seq));
    let (pointer_events, mouse_frames, keyboard_frames) =
        drain_pending_input_frames(runtime_state, runtime_stats.as_ref());
    let now_ms = now_ms_f64();
    let packet = build_input_stream_packet(
        *sequence,
        now_ms,
        metadata.as_ref(),
        frames,
        &pointer_events,
        &mouse_frames,
        &keyboard_frames,
    );
    if input_channel.send(&Bytes::from(packet)).await.is_ok() {
        *sequence = sequence.wrapping_add(1);
        *last_input_packet_sent_at_ms = now_ms;
        *last_sample_count = sample_count;
        last_sample_signature[..sample_count].copy_from_slice(&sample_signature[..sample_count]);
    }
}

async fn handle_input_channel_binary_message(payload: &[u8]) {
    let Some(request) = parse_input_rumble_packet(payload) else {
        return;
    };
    let Ok(host) = GamepadRuntimeHost::shared() else {
        return;
    };
    let _ = host.play_rumble(request);
}

fn reset_state_on_channel_close(runtime_state: &Arc<Mutex<XbxDataChannelState>>, label: &str) {
    let Ok(mut state) = runtime_state.lock() else {
        return;
    };
    // channel close 后允许重启对应子流程，避免网络抖动后卡在“已启动”状态。
    match label {
        "message" => {
            state.message_handshake_acked = false;
            state.control_started = false;
            state.control_bootstrapped_after_handshake = false;
            state.chat_open = false;
            state.input_metadata_sent = false;
            state.input_metadata_bootstrapped_after_handshake = false;
            state.input_stream_loop_started = false;
            state.pending_input_events.clear();
            abort_task_handle(&mut state.control_gamepad_added_task);
            abort_task_handle(&mut state.control_keyframe_prime_task);
            abort_task_handle(&mut state.input_stream_loop_task);
        }
        "chat" => {
            state.chat_open = false;
        }
        "control" => {
            state.control_started = false;
            state.control_bootstrapped_after_handshake = false;
            state.control_channel = None;
            abort_task_handle(&mut state.control_gamepad_added_task);
            abort_task_handle(&mut state.control_keyframe_prime_task);
        }
        "input" => {
            state.input_metadata_sent = false;
            state.input_metadata_bootstrapped_after_handshake = false;
            state.input_stream_loop_started = false;
            state.pending_input_events.clear();
            abort_task_handle(&mut state.input_stream_loop_task);
        }
        _ => {}
    }
}

fn replace_task_handle(slot: &mut Option<JoinHandle<()>>, next: JoinHandle<()>) {
    if let Some(previous) = slot.replace(next) {
        previous.abort();
    }
}

fn abort_task_handle(slot: &mut Option<JoinHandle<()>>) {
    if let Some(handle) = slot.take() {
        handle.abort();
    }
}

// 协议常量 (从已删除的 network_profile.rs 迁移)
const STREAM_INPUT_POLL_INTERVAL_MS: u64 = 8;
const STREAM_INPUT_MAX_BUFFERED_AMOUNT_BYTES: u64 = 1024;
const STREAM_INPUT_IDLE_GAMEPAD_KEEPALIVE_MS: u64 = 250;
const STREAM_CONTROL_GAMEPAD_ADDED_DELAY_MS: u64 = 500;
const STREAM_CONTROL_KEYFRAME_PRIME_DELAY_MS: u64 = 300;
const STREAM_INPUT_INITIAL_MAX_TOUCHPOINTS: u8 = 64;

const REPORT_TYPE_METADATA: u16 = 1;
const REPORT_TYPE_GAMEPAD: u16 = 2;
const REPORT_TYPE_POINTER: u16 = 4;
const REPORT_TYPE_CLIENT_METADATA: u16 = 8;
const REPORT_TYPE_MOUSE: u16 = 32;
const REPORT_TYPE_KEYBOARD: u16 = 64;
const REPORT_TYPE_VIBRATION: u16 = 128;

const MESSAGE_HANDSHAKE_ID: &str = "f9c5f412-0e69-4ede-8e62-92c7f5358c56";
const MESSAGE_TRANSACTION_ID: &str = "41f93d5a-900f-4d33-b7a1-2d4ca6747072";
const MESSAGE_CLIENT_APP_INSTALL_ID: &str = "c11ddb2e-c7e3-4f02-a62b-fd5448e0b851";
const CONTROL_ACCESS_KEY: &str = "4BDB3609-C1F1-4195-9B37-FEFF45DA8B8E";
const DEFAULT_VIEWPORT_WIDTH: u32 = 1920;
const DEFAULT_VIEWPORT_HEIGHT: u32 = 1080;

fn build_message_handshake_payload() -> String {
    serde_json::json!({
        "type": "Handshake",
        "version": "messageV1",
        "id": MESSAGE_HANDSHAKE_ID,
        "cv": "",
    })
    .to_string()
}

fn build_post_handshake_message_payloads() -> Vec<String> {
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

fn is_handshake_ack_payload(payload: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|json| {
            json.get("type")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .is_some_and(|value| value == "HandshakeAck")
}

fn build_control_keyframe_request_payload() -> String {
    serde_json::json!({
        "message": "videoKeyframeRequested",
        "ifrRequested": true,
    })
    .to_string()
}

fn build_control_decoder_reset_payload() -> String {
    serde_json::json!({
        "message": "decoderReset",
    })
    .to_string()
}

fn build_control_authorization_payload() -> String {
    serde_json::json!({
        "message": "authorizationRequest",
        "accessKey": CONTROL_ACCESS_KEY,
    })
    .to_string()
}

fn build_control_gamepad_changed_payload(added: bool) -> String {
    serde_json::json!({
        "message": "gamepadChanged",
        "gamepadIndex": 0,
        "wasAdded": added,
    })
    .to_string()
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

fn build_input_metadata_packet(seq: u32, time: f64, max_touchpoints: u8) -> Vec<u8> {
    let mut packet = Vec::with_capacity(15);
    packet.extend_from_slice(&REPORT_TYPE_CLIENT_METADATA.to_le_bytes());
    packet.extend_from_slice(&seq.to_le_bytes());
    packet.extend_from_slice(&time.to_le_bytes());
    packet.push(max_touchpoints);
    packet
}

fn build_input_stream_packet(
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
        packet.push(logical_pad_index(&frame.pad_id));
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

fn drain_pending_input_frames(
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
        .unwrap_or((DEFAULT_VIEWPORT_WIDTH, DEFAULT_VIEWPORT_HEIGHT));
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

fn parse_input_rumble_packet(payload: &[u8]) -> Option<OhMyGamepadRumbleRequestDto> {
    parse_better_xcloud_rumble_packet(payload).or_else(|| parse_legacy_rumble_packet(payload))
}

fn parse_better_xcloud_rumble_packet(payload: &[u8]) -> Option<OhMyGamepadRumbleRequestDto> {
    if payload.len() < 13 {
        return None;
    }

    let mut offset = 0usize;
    let mut message_type = payload[offset] as u16;
    let mut message_type_size = 1usize;
    let v8_message_type = u16::from_le_bytes([payload[0], payload[1]]);
    if (v8_message_type & REPORT_TYPE_VIBRATION) != 0 && payload.len() >= 13 {
        message_type = v8_message_type;
        message_type_size = 2;
    }
    if (message_type & REPORT_TYPE_VIBRATION) == 0 {
        return None;
    }

    offset += message_type_size;
    let vibration_type = *payload.get(offset)?;
    offset += 1;
    if vibration_type != 0 {
        return None;
    }

    let gamepad_index = *payload.get(offset)?;
    let effect = OhMyGamepadRumbleEffectDto {
        strong_magnitude: normalize_percent_motor(*payload.get(offset + 1)?),
        weak_magnitude: normalize_percent_motor(*payload.get(offset + 2)?),
        left_trigger: normalize_percent_motor(*payload.get(offset + 3)?),
        right_trigger: normalize_percent_motor(*payload.get(offset + 4)?),
        duration_ms: u16::from_le_bytes([*payload.get(offset + 5)?, *payload.get(offset + 6)?]),
        start_delay_ms: 0,
        repeat: 0,
    };

    Some(OhMyGamepadRumbleRequestDto {
        target: logical_pad_target_from_index(gamepad_index),
        effect,
    })
}

fn parse_legacy_rumble_packet(payload: &[u8]) -> Option<OhMyGamepadRumbleRequestDto> {
    if payload.len() < 14 || payload.first().copied()? as u16 != REPORT_TYPE_VIBRATION {
        return None;
    }

    let offset = 2usize;
    let gamepad_index = *payload.get(offset + 1)?;
    let effect = OhMyGamepadRumbleEffectDto {
        left_trigger: normalize_legacy_motor(u16::from_le_bytes([
            *payload.get(offset + 2)?,
            *payload.get(offset + 3)?,
        ])),
        right_trigger: normalize_legacy_motor(u16::from_le_bytes([
            *payload.get(offset + 4)?,
            *payload.get(offset + 5)?,
        ])),
        weak_magnitude: normalize_legacy_motor(u16::from_le_bytes([
            *payload.get(offset + 6)?,
            *payload.get(offset + 7)?,
        ])),
        strong_magnitude: normalize_legacy_motor(u16::from_le_bytes([
            *payload.get(offset + 8)?,
            *payload.get(offset + 9)?,
        ])),
        duration_ms: u16::from_le_bytes([*payload.get(offset + 10)?, *payload.get(offset + 11)?]),
        start_delay_ms: 0,
        repeat: 0,
    };

    Some(OhMyGamepadRumbleRequestDto {
        target: logical_pad_target_from_index(gamepad_index),
        effect,
    })
}

fn logical_pad_target_from_index(gamepad_index: u8) -> OhMyGamepadRumbleTargetDto {
    match gamepad_index {
        0 => OhMyGamepadRumbleTargetDto::LogicalPad {
            pad_id: LogicalPadId::Pad0,
        },
        1 => OhMyGamepadRumbleTargetDto::LogicalPad {
            pad_id: LogicalPadId::Pad1,
        },
        2 => OhMyGamepadRumbleTargetDto::LogicalPad {
            pad_id: LogicalPadId::Pad2,
        },
        3 => OhMyGamepadRumbleTargetDto::LogicalPad {
            pad_id: LogicalPadId::Pad3,
        },
        _ => OhMyGamepadRumbleTargetDto::Auto,
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

fn normalize_percent_motor(value: u8) -> f32 {
    (value as f32).clamp(0.0, 100.0) / 100.0
}

fn normalize_legacy_motor(value: u16) -> f32 {
    (value as f32 / 1023.0).clamp(0.0, 1.0)
}

#[derive(Clone, Copy)]
struct VideoMetadataFrame {
    server_data_key: u32,
    first_frame_packet_arrival_time_ms: u32,
    frame_submitted_time_ms: u32,
    frame_decoded_time_ms: u32,
    frame_rendered_time_ms: u32,
}

fn build_metadata_frame(
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

fn clamp_u32_ms(value: f64) -> u32 {
    value.max(0.0).round().clamp(0.0, u32::MAX as f64) as u32
}

fn now_ms_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}
