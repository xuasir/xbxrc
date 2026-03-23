use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ohmygamepad_host::GamepadRuntimeHost;
use ohmygamepad_protocol::{LogicalPadSnapshotDto, OhMyGamepadRouteTargetDto};
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration};

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::protocol::data_channel_state::{
    build_input_stream_packet, build_metadata_frame, drain_pending_input_frames,
    XbxDataChannelState, STREAM_INPUT_IDLE_GAMEPAD_KEEPALIVE_MS,
};

// 轮询任务单独封装，避免 stack.rs 继续承载输入流的调度细节。
const RTC_INPUT_STREAM_POLL_INTERVAL_MS: u64 = 8;

#[derive(Clone, Debug)]
struct RtcInputStreamState {
    input_sequence: u32,
    last_input_packet_sent_at_ms: f64,
    last_gamepad_sample_count: usize,
    last_gamepad_sample_signature: [u64; 4],
    gamepad_sample_signature: [u64; 4],
    last_metadata_frame_seq: u64,
}

impl Default for RtcInputStreamState {
    fn default() -> Self {
        Self {
            input_sequence: 1,
            last_input_packet_sent_at_ms: 0.0,
            last_gamepad_sample_count: 0,
            last_gamepad_sample_signature: [0; 4],
            gamepad_sample_signature: [0; 4],
            last_metadata_frame_seq: 0,
        }
    }
}

pub(crate) struct RtcInputStreamController {
    media_runtime: Arc<tokio::runtime::Runtime>,
    connection: Arc<Mutex<RtcConnectionService>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    data_channel_state: Arc<Mutex<XbxDataChannelState>>,
    input_stream_state: Arc<Mutex<RtcInputStreamState>>,
    input_loop_stop: Arc<AtomicBool>,
    input_loop_task: Option<JoinHandle<()>>,
}

impl RtcInputStreamController {
    pub(crate) fn new(
        media_runtime: Arc<tokio::runtime::Runtime>,
        connection: Arc<Mutex<RtcConnectionService>>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        data_channel_state: Arc<Mutex<XbxDataChannelState>>,
    ) -> Self {
        Self {
            media_runtime,
            connection,
            runtime_stats,
            data_channel_state,
            input_stream_state: Arc::new(Mutex::new(RtcInputStreamState::default())),
            input_loop_stop: Arc::new(AtomicBool::new(false)),
            input_loop_task: None,
        }
    }

    pub(crate) fn ensure_running(&mut self) {
        if self
            .input_loop_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        self.input_loop_stop.store(false, Ordering::Relaxed);
        let connection = self.connection.clone();
        let runtime_stats = self.runtime_stats.clone();
        let data_channel_state = self.data_channel_state.clone();
        let input_stream_state = self.input_stream_state.clone();
        let stop = self.input_loop_stop.clone();
        let task = self.media_runtime.spawn(async move {
            let mut ticker = interval(Duration::from_millis(RTC_INPUT_STREAM_POLL_INTERVAL_MS));
            ticker.tick().await;
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                ticker.tick().await;
                Self::pump_input_stream_once(
                    &connection,
                    &runtime_stats,
                    &data_channel_state,
                    &input_stream_state,
                );
            }
        });
        self.input_loop_task = Some(task);
    }

    pub(crate) fn reset_state(&self) {
        if let Ok(mut input_state) = self.input_stream_state.lock() {
            *input_state = RtcInputStreamState::default();
        }
    }

    pub(crate) fn stop(&mut self) {
        self.input_loop_stop.store(true, Ordering::Relaxed);
        if let Some(task) = self.input_loop_task.take() {
            task.abort();
        }
    }

    fn collect_gamepad_frames(input_state: &mut RtcInputStreamState) -> Vec<LogicalPadSnapshotDto> {
        let Ok(host) = GamepadRuntimeHost::shared() else {
            return Vec::new();
        };
        let Ok(snapshot) = host.snapshot() else {
            return Vec::new();
        };
        if !matches!(
            snapshot.route_target,
            OhMyGamepadRouteTargetDto::StreamSession { .. }
        ) {
            return Vec::new();
        }
        let mut frames = Vec::with_capacity(4);
        let mut sample_count = 0usize;
        for frame in snapshot.pads.iter().take(4) {
            if sample_count < input_state.gamepad_sample_signature.len() {
                input_state.gamepad_sample_signature[sample_count] = frame.sample_seq;
            }
            sample_count += 1;
            frames.push(frame.clone());
        }
        input_state.gamepad_sample_signature[sample_count..].fill(0);
        frames
    }

    fn pump_input_stream_once(
        connection: &Arc<Mutex<RtcConnectionService>>,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        data_channel_state: &Arc<Mutex<XbxDataChannelState>>,
        input_stream_state: &Arc<Mutex<RtcInputStreamState>>,
    ) {
        let Ok(mut input_state) = input_stream_state.lock() else {
            return;
        };
        let (metadata, pointer_events, mouse_frames, keyboard_frames, frames, now_ms) = {
            let metadata = runtime_stats.lock().ok().and_then(|stats| {
                build_metadata_frame(&stats, &mut input_state.last_metadata_frame_seq)
            });
            let frames = Self::collect_gamepad_frames(&mut input_state);
            let sample_count = frames.len();
            let (pointer_events, mouse_frames, keyboard_frames) =
                drain_pending_input_frames(data_channel_state, runtime_stats);
            let gamepad_changed = sample_count > 0
                && !(sample_count == input_state.last_gamepad_sample_count
                    && input_state.gamepad_sample_signature[..sample_count]
                        == input_state.last_gamepad_sample_signature
                            [..input_state.last_gamepad_sample_count]);
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            let should_send_idle_gamepad_keepalive = sample_count > 0
                && !gamepad_changed
                && pointer_events.is_empty()
                && mouse_frames.is_empty()
                && keyboard_frames.is_empty()
                && (now_ms - input_state.last_input_packet_sent_at_ms)
                    >= STREAM_INPUT_IDLE_GAMEPAD_KEEPALIVE_MS as f64;
            if metadata.is_none()
                && !gamepad_changed
                && !should_send_idle_gamepad_keepalive
                && pointer_events.is_empty()
                && mouse_frames.is_empty()
                && keyboard_frames.is_empty()
            {
                return;
            }
            let gamepad_frames = if gamepad_changed || should_send_idle_gamepad_keepalive {
                frames
            } else {
                Vec::new()
            };
            (
                metadata,
                pointer_events,
                mouse_frames,
                keyboard_frames,
                gamepad_frames,
                now_ms,
            )
        };

        let packet = build_input_stream_packet(
            input_state.input_sequence,
            now_ms,
            metadata.as_ref(),
            &frames,
            &pointer_events,
            &mouse_frames,
            &keyboard_frames,
        );

        let sent = connection
            .lock()
            .ok()
            .and_then(|mut connection| {
                connection
                    .send_input_stream_packet(packet, runtime_stats)
                    .ok()
            })
            .unwrap_or(false);
        if !sent {
            return;
        }
        input_state.input_sequence = input_state.input_sequence.wrapping_add(1);
        input_state.last_input_packet_sent_at_ms = now_ms;
        if !frames.is_empty() {
            input_state.last_gamepad_sample_count = frames.len();
            let current_signature = input_state.gamepad_sample_signature;
            input_state.last_gamepad_sample_signature[..frames.len()]
                .copy_from_slice(&current_signature[..frames.len()]);
            input_state.last_gamepad_sample_signature[frames.len()..].fill(0);
        }
    }
}
