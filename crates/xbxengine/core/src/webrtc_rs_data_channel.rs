use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use ohmygamepad_host::GamepadRuntimeHost;
use ohmygamepad_protocol::OhMyGamepadRouteTargetDto;
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, Duration};
use webrtc::data_channel::{data_channel_state::RTCDataChannelState, RTCDataChannel};

use crate::{
    network_profile::{
        build_control_authorization_payload, build_control_gamepad_changed_payload,
        build_control_keyframe_request_payload, build_input_gamepad_packet,
        build_input_metadata_packet, build_message_handshake_payload,
        build_post_handshake_message_payloads, is_handshake_ack_payload, parse_input_rumble_packet,
        STREAM_CONTROL_GAMEPAD_ADDED_DELAY_MS, STREAM_CONTROL_KEYFRAME_INTERVAL_MS,
        STREAM_INPUT_MAX_BUFFERED_AMOUNT_BYTES, STREAM_INPUT_POLL_INTERVAL_MS,
    },
    XbxEngineRuntimeError,
};

#[derive(Default)]
pub(crate) struct WebRtcRsDataChannelState {
    message_handshake_acked: bool,
    control_started: bool,
    control_keyframe_loop_started: bool,
    input_metadata_sent: bool,
    input_stream_loop_started: bool,
    control_gamepad_added_task: Option<JoinHandle<()>>,
    control_keyframe_loop_task: Option<JoinHandle<()>>,
    input_stream_loop_task: Option<JoinHandle<()>>,
}

pub(crate) fn install_data_channel_contracts(
    data_channels: &BTreeMap<String, Arc<RTCDataChannel>>,
    runtime_state: Arc<Mutex<WebRtcRsDataChannelState>>,
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

    for (label, channel) in data_channels {
        let label = label.clone();
        let label_for_open = label.clone();
        let runtime_state_for_open = runtime_state.clone();
        let runtime_state_for_close = runtime_state.clone();
        let message_channel_for_open = message_channel.clone();
        let control_channel_for_open = control_channel.clone();
        let input_channel_for_open = input_channel.clone();
        let chat_channel_for_open = chat_channel.clone();
        channel.on_open(Box::new(move || {
            let label = label_for_open.clone();
            let runtime_state = runtime_state_for_open.clone();
            let message_channel = message_channel_for_open.clone();
            let control_channel = control_channel_for_open.clone();
            let input_channel = input_channel_for_open.clone();
            let chat_channel = chat_channel_for_open.clone();
            Box::pin(async move {
                eprintln!("[xbxengine][webrtc-rs] data channel open label={label}");
                if label == "message" {
                    let _ = message_channel
                        .send_text(build_message_handshake_payload())
                        .await;
                } else if label == "chat" {
                    let _ = chat_channel;
                } else if label == "control" || label == "input" {
                    bootstrap_post_handshake_channels(
                        runtime_state,
                        message_channel,
                        control_channel,
                        input_channel,
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
                eprintln!("[xbxengine][webrtc-rs] data channel close label={label}");
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
                            eprintln!(
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
    message_channel.on_message(Box::new(move |message| {
        let runtime_state = runtime_state.clone();
        let message_channel = message_channel_for_message.clone();
        let control_channel = control_channel_for_message.clone();
        let input_channel = input_channel_for_message.clone();
        Box::pin(async move {
            if !message.is_string {
                return;
            }
            let Ok(payload) = String::from_utf8(message.data.to_vec()) else {
                return;
            };
            eprintln!(
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

async fn bootstrap_post_handshake_channels(
    runtime_state: Arc<Mutex<WebRtcRsDataChannelState>>,
    message_channel: Arc<RTCDataChannel>,
    control_channel: Arc<RTCDataChannel>,
    input_channel: Arc<RTCDataChannel>,
) {
    let mut should_start_control = false;
    let mut should_start_control_keyframe_loop = false;
    let mut should_send_input_metadata = false;
    let mut should_start_input_stream_loop = false;

    if let Ok(mut state) = runtime_state.lock() {
        if !state.message_handshake_acked {
            return;
        }
        if !state.control_started && control_channel.ready_state() == RTCDataChannelState::Open {
            state.control_started = true;
            should_start_control = true;
        }
        if !state.control_keyframe_loop_started
            && control_channel.ready_state() == RTCDataChannelState::Open
        {
            state.control_keyframe_loop_started = true;
            should_start_control_keyframe_loop = true;
        }
        if !state.input_metadata_sent && input_channel.ready_state() == RTCDataChannelState::Open {
            state.input_metadata_sent = true;
            should_send_input_metadata = true;
        }
        if !state.input_stream_loop_started
            && input_channel.ready_state() == RTCDataChannelState::Open
        {
            state.input_stream_loop_started = true;
            should_start_input_stream_loop = true;
        }
    }

    if should_start_control {
        let _ = control_channel
            .send_text(build_control_authorization_payload())
            .await;
        let _ = control_channel
            .send_text(build_control_gamepad_changed_payload(false))
            .await;
        // 首次 bootstrap 后立即请求一次关键帧，避免首帧等待周期任务触发。
        let _ = control_channel
            .send_text(build_control_keyframe_request_payload())
            .await;
        let delayed_added_task = start_delayed_gamepad_added(control_channel.clone());
        if let Ok(mut state) = runtime_state.lock() {
            replace_task_handle(&mut state.control_gamepad_added_task, delayed_added_task);
        }
    }
    if should_start_control_keyframe_loop {
        let keyframe_loop_task = start_periodic_keyframe_loop(control_channel);
        if let Ok(mut state) = runtime_state.lock() {
            replace_task_handle(&mut state.control_keyframe_loop_task, keyframe_loop_task);
        }
    }
    if should_send_input_metadata {
        let _ = input_channel
            .send(&Bytes::from(build_input_metadata_packet(0, now_ms_f64())))
            .await;
    }
    if should_start_input_stream_loop {
        let input_stream_loop_task = start_input_stream_loop(input_channel);
        if let Ok(mut state) = runtime_state.lock() {
            replace_task_handle(&mut state.input_stream_loop_task, input_stream_loop_task);
        }
    }

    let _ = message_channel;
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

fn start_periodic_keyframe_loop(control_channel: Arc<RTCDataChannel>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(STREAM_CONTROL_KEYFRAME_INTERVAL_MS));
        // tokio interval 首次 tick 会立即返回；这里先消费一次，保持和 setInterval 一致。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if control_channel.ready_state() != RTCDataChannelState::Open {
                break;
            }
            let _ = control_channel
                .send_text(build_control_keyframe_request_payload())
                .await;
        }
    })
}

fn start_input_stream_loop(input_channel: Arc<RTCDataChannel>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Ok(host) = GamepadRuntimeHost::shared() else {
            return;
        };
        let mut ticker = interval(Duration::from_millis(STREAM_INPUT_POLL_INTERVAL_MS));
        // 与 web input 定时器对齐：首包在一个 polling 周期后发送。
        ticker.tick().await;
        let mut sequence = 1u32;
        let mut last_sample_count = 0usize;
        let mut last_sample_signature = [0u64; 4];
        let mut sample_signature = [0u64; 4];
        let mut frames = Vec::with_capacity(4);

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
            if sample_count == 0 {
                continue;
            }
            if sample_count == last_sample_count
                && sample_signature[..sample_count] == last_sample_signature[..last_sample_count]
            {
                continue;
            }

            // channel 拥塞时丢旧帧，避免 buffered amount 持续上升导致输入延迟。
            if input_channel.buffered_amount().await > STREAM_INPUT_MAX_BUFFERED_AMOUNT_BYTES {
                continue;
            }

            let packet = build_input_gamepad_packet(sequence, now_ms_f64(), &frames);
            if input_channel.send(&Bytes::from(packet)).await.is_ok() {
                sequence = sequence.wrapping_add(1);
                last_sample_count = sample_count;
                last_sample_signature[..sample_count]
                    .copy_from_slice(&sample_signature[..sample_count]);
            }
        }
    })
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

fn reset_state_on_channel_close(runtime_state: &Arc<Mutex<WebRtcRsDataChannelState>>, label: &str) {
    let Ok(mut state) = runtime_state.lock() else {
        return;
    };
    // channel close 后允许重启对应子流程，避免网络抖动后卡在“已启动”状态。
    match label {
        "message" => {
            state.message_handshake_acked = false;
            state.control_started = false;
            state.control_keyframe_loop_started = false;
            state.input_metadata_sent = false;
            state.input_stream_loop_started = false;
            abort_task_handle(&mut state.control_gamepad_added_task);
            abort_task_handle(&mut state.control_keyframe_loop_task);
            abort_task_handle(&mut state.input_stream_loop_task);
        }
        "control" => {
            state.control_started = false;
            state.control_keyframe_loop_started = false;
            abort_task_handle(&mut state.control_gamepad_added_task);
            abort_task_handle(&mut state.control_keyframe_loop_task);
        }
        "input" => {
            state.input_metadata_sent = false;
            state.input_stream_loop_started = false;
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

fn now_ms_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}
