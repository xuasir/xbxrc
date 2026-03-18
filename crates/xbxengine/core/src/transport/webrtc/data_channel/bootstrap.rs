use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use ohmygamepad_host::GamepadRuntimeHost;
use ohmygamepad_protocol::{LogicalPadSnapshotDto, OhMyGamepadRouteTargetDto};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, Duration};
use webrtc::data_channel::{data_channel_state::RTCDataChannelState, RTCDataChannel};

use crate::transport::webrtc::data_channel::{
    build_control_authorization_payload, build_control_gamepad_changed_payload,
    build_control_keyframe_request_payload, build_input_metadata_packet, build_input_stream_packet,
    build_message_handshake_payload, build_metadata_frame, build_post_handshake_message_payloads,
    catalog_data_channel_message, drain_pending_input_frames, is_handshake_ack_payload,
    recovery_requests_ready, request_decoder_reset_on_control_channel,
    request_video_keyframe_on_control_channel, XbxDataChannelState,
    STREAM_CONTROL_GAMEPAD_ADDED_DELAY_MS, STREAM_CONTROL_KEYFRAME_PRIME_DELAY_MS,
    STREAM_INPUT_IDLE_GAMEPAD_KEEPALIVE_MS, STREAM_INPUT_INITIAL_MAX_TOUCHPOINTS,
    STREAM_INPUT_MAX_BUFFERED_AMOUNT_BYTES, STREAM_INPUT_POLL_INTERVAL_MS,
};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError};

/**
 * data channel bootstrap/lifecycle：
 * - handshake 后的 control/input 拉起
 * - pending recovery request 回放
 * - input loop / keepalive / delayed prime
 * - channel close 后的局部状态复位
 */
pub(crate) fn install_data_channel_contracts(
    data_channels: &std::collections::BTreeMap<String, Arc<RTCDataChannel>>,
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
        let runtime_state_for_message = runtime_state.clone();
        let message_channel_for_open = message_channel.clone();
        let control_channel_for_open = control_channel.clone();
        let input_channel_for_open = input_channel.clone();
        let chat_channel_for_open = chat_channel.clone();
        let runtime_stats_for_open = runtime_stats.clone();
        let runtime_stats_for_message = runtime_stats.clone();
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
                    let payload = build_message_handshake_payload();
                    catalog_data_channel_message(
                        &runtime_state,
                        &runtime_stats,
                        "outbound",
                        "message",
                        &payload,
                    );
                    let _ = message_channel.send_text(payload).await;
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
                let runtime_state = runtime_state_for_message.clone();
                let runtime_stats = runtime_stats_for_message.clone();
                Box::pin(async move {
                    if label == "input" && !message.is_string {
                        super::handle_input_channel_binary_message(&message.data).await;
                        return;
                    }

                    if message.is_string {
                        if let Ok(payload) = String::from_utf8(message.data.to_vec()) {
                            if label == "control" || label == "message" {
                                catalog_data_channel_message(
                                    &runtime_state,
                                    &runtime_stats,
                                    "inbound",
                                    if label == "control" {
                                        "control"
                                    } else {
                                        "message"
                                    },
                                    &payload,
                                );
                            }
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
                    catalog_data_channel_message(
                        &runtime_state,
                        &runtime_stats,
                        "outbound",
                        "message",
                        &payload,
                    );
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
                || (message_handshake_acked && !state.input_metadata_bootstrapped_after_handshake))
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
        if !message_handshake_acked {
            crate::xbx_log_warn!(
                "[xbxengine][webrtc-rs] bootstrap control before message handshake ack"
            );
        } else {
            crate::xbx_log_info!(
                "[xbxengine][webrtc-rs] bootstrap control after message handshake ack"
            );
        }
        let authorization_payload = build_control_authorization_payload();
        catalog_data_channel_message(
            &runtime_state,
            &runtime_stats,
            "outbound",
            "control",
            &authorization_payload,
        );
        let _ = control_channel.send_text(authorization_payload).await;
        let removed_payload = build_control_gamepad_changed_payload(false);
        catalog_data_channel_message(
            &runtime_state,
            &runtime_stats,
            "outbound",
            "control",
            &removed_payload,
        );
        let _ = control_channel.send_text(removed_payload).await;
        let protocol_ready = runtime_state
            .lock()
            .ok()
            .is_some_and(|state| is_recovery_request_actionable(&state));
        if protocol_ready {
            // 恢复请求和默认 prime 都统一收口到“协议 ready”之后，
            // 避免 control 先开时把关键请求打在 handshake 之前。
            let flushed_pending_recovery =
                flush_pending_recovery_requests(runtime_state.clone(), control_channel.clone())
                    .await;
            if !flushed_pending_recovery {
                let keyframe_payload = build_control_keyframe_request_payload();
                catalog_data_channel_message(
                    &runtime_state,
                    &runtime_stats,
                    "outbound",
                    "control",
                    &keyframe_payload,
                );
                let _ = control_channel.send_text(keyframe_payload).await;
            }
        }
        let delayed_added_task =
            start_delayed_gamepad_added(runtime_state.clone(), control_channel.clone());
        let delayed_keyframe_prime_task =
            start_delayed_keyframe_prime(runtime_state.clone(), control_channel.clone());
        if let Ok(mut state) = runtime_state.lock() {
            replace_task_handle(&mut state.control_gamepad_added_task, delayed_added_task);
            replace_task_handle(
                &mut state.control_keyframe_prime_task,
                delayed_keyframe_prime_task,
            );
        }
    }
    if !should_bootstrap_control {
        let protocol_ready = runtime_state
            .lock()
            .ok()
            .is_some_and(|state| is_recovery_request_actionable(&state));
        if protocol_ready {
            let _ = flush_pending_recovery_requests(runtime_state.clone(), control_channel.clone())
                .await;
        }
    }
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
) -> bool {
    if control_channel.ready_state() != RTCDataChannelState::Open {
        return false;
    }

    let (flush_decoder_reset, flush_keyframe) = {
        let Ok(state) = runtime_state.lock() else {
            return false;
        };
        (
            state.pending_decoder_reset,
            state.pending_keyframe_request && !state.pending_decoder_reset,
        )
    };

    if flush_decoder_reset {
        match request_decoder_reset_on_control_channel(&runtime_state, &control_channel).await {
            Ok(()) => {
                if let Ok(mut state) = runtime_state.lock() {
                    state.pending_decoder_reset = false;
                    state.pending_keyframe_request = false;
                }
                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] flushed pending decoder reset after control channel recovery"
                );
                return true;
            }
            Err(error) => {
                crate::xbx_log_warn!(
                    "[xbxengine][webrtc-rs] flush pending decoder reset failed: {error}"
                );
            }
        }
        return false;
    }

    if flush_keyframe {
        match request_video_keyframe_on_control_channel(&runtime_state, &control_channel).await {
            Ok(()) => {
                if let Ok(mut state) = runtime_state.lock() {
                    state.pending_keyframe_request = false;
                }
                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] flushed pending keyframe request after control channel recovery"
                );
                return true;
            }
            Err(error) => {
                crate::xbx_log_warn!(
                    "[xbxengine][webrtc-rs] flush pending keyframe request failed: {error}"
                );
            }
        }
    }

    false
}

fn start_delayed_gamepad_added(
    _runtime_state: Arc<Mutex<XbxDataChannelState>>,
    control_channel: Arc<RTCDataChannel>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        sleep(Duration::from_millis(STREAM_CONTROL_GAMEPAD_ADDED_DELAY_MS)).await;
        if control_channel.ready_state() != RTCDataChannelState::Open {
            return;
        }
        let _ = control_channel
            .send_text(build_control_gamepad_changed_payload(true))
            .await;
    })
}

fn start_delayed_keyframe_prime(
    runtime_state: Arc<Mutex<XbxDataChannelState>>,
    control_channel: Arc<RTCDataChannel>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        sleep(Duration::from_millis(
            STREAM_CONTROL_KEYFRAME_PRIME_DELAY_MS,
        ))
        .await;
        if control_channel.ready_state() != RTCDataChannelState::Open {
            return;
        }
        let protocol_ready = runtime_state
            .lock()
            .ok()
            .is_some_and(|state| is_recovery_request_actionable(&state));
        if !protocol_ready {
            if let Ok(mut state) = runtime_state.lock() {
                state.pending_keyframe_request = true;
            }
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

pub(crate) fn reset_state_on_channel_close(
    runtime_state: &Arc<Mutex<XbxDataChannelState>>,
    label: &str,
) {
    let Ok(mut state) = runtime_state.lock() else {
        return;
    };
    match label {
        "message" => {
            state.message_handshake_acked = false;
            state.control_started = false;
            state.control_bootstrapped_after_handshake = false;
            state.chat_open = false;
            state.seen_message_catalog_keys.clear();
            state.next_message_catalog_observation_id = 0;
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

fn now_ms_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn is_recovery_request_actionable(state: &XbxDataChannelState) -> bool {
    if recovery_requests_ready(state) {
        return true;
    }
    let control_channel_open = state
        .control_channel
        .as_ref()
        .is_some_and(|channel| channel.ready_state() == RTCDataChannelState::Open);
    is_startup_recovery_fallback_ready(
        state.message_handshake_acked,
        state.control_started,
        control_channel_open,
    )
}

fn is_startup_recovery_fallback_ready(
    message_handshake_acked: bool,
    control_started: bool,
    control_channel_open: bool,
) -> bool {
    // 启动期兜底：若 message handshake 迟迟未 ack，但 control 已经 open，
    // 仍允许恢复请求先发送，避免早期卡在 pending 队列。
    !message_handshake_acked && control_started && control_channel_open
}

#[cfg(test)]
mod tests {
    use super::is_startup_recovery_fallback_ready;

    #[test]
    fn startup_fallback_allows_recovery_when_handshake_not_acked_and_control_open() {
        assert!(is_startup_recovery_fallback_ready(false, true, true));
    }

    #[test]
    fn startup_fallback_blocks_recovery_when_handshake_already_acked() {
        assert!(!is_startup_recovery_fallback_ready(true, true, true));
    }

    #[test]
    fn startup_fallback_blocks_recovery_when_control_not_ready() {
        assert!(!is_startup_recovery_fallback_ready(false, false, true));
        assert!(!is_startup_recovery_fallback_ready(false, true, false));
    }
}
