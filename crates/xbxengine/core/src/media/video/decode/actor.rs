use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;

use crate::media::video::decode::video_decode::XbxVideoDecodeState;
use crate::media::video::pacer::actor::PacerActorHandle;
use crate::media::video::types::{DecodedFrame, EncodedFrame};
use crate::XbxEngineMediaRuntimeStats;
use std::sync::Arc;
use std::sync::Mutex;

const DECODER_STALL_PACKET_FRESH_MAX_AGE_MS: f64 = 400.0;
const DECODER_STALL_DECODE_AGE_MS: f64 = 1_000.0;

pub enum DecodeMsg {
    Frame(EncodedFrame),
    Flush,
    Stop,
}

pub struct DecodeActorHandle {
    tx: SyncSender<DecodeMsg>,
}

impl DecodeActorHandle {
    pub fn new(
        pacer: Arc<PacerActorHandle>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        min_delay_ms: u64,
        max_delay_ms: u64,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel(2);

        thread::Builder::new()
            .name("XbxDecodeActor".into())
            .spawn(move || {
                run_decode_loop(rx, pacer, runtime_stats, min_delay_ms, max_delay_ms);
            })
            .expect("Failed to spawn decode actor thread");

        Self { tx }
    }

    pub fn submit(&self, frame: EncodedFrame) -> Result<(), TrySendError<DecodeMsg>> {
        match self.tx.try_send(DecodeMsg::Frame(frame)) {
            Ok(_) => Ok(()),
            Err(e) => match e {
                TrySendError::Full(_) => Err(e),
                TrySendError::Disconnected(_) => {
                    crate::xbx_log_error!(
                        "[DecodeActorHandle] Decode thread is disconnected (likely panicked)!"
                    );
                    Err(e)
                }
            },
        }
    }

    pub fn flush(&self) {
        let _ = self.tx.send(DecodeMsg::Flush);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(DecodeMsg::Stop);
    }
}

fn run_decode_loop(
    rx: Receiver<DecodeMsg>,
    pacer: Arc<PacerActorHandle>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    min_delay_ms: u64,
    max_delay_ms: u64,
) {
    // 设置线程局部的 panic hook，确保崩溃信息能被记录到 xbx_log
    std::panic::set_hook(Box::new(|panic_info| {
        let msg = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            &s[..]
        } else {
            "Box<Any>"
        };
        crate::xbx_log_error!(
            "[XbxDecodeActor] PANIC occurred: {} at {:?}",
            msg,
            panic_info.location()
        );
    }));

    let mut decode_state = match XbxVideoDecodeState::new(min_delay_ms, max_delay_ms) {
        Ok(state) => state,
        Err(e) => {
            crate::xbx_log_error!("Failed to initialize hardware decoder: {:?}", e);
            return;
        }
    };
    let mut recent_decode_times_ms = std::collections::VecDeque::<f64>::new();
    sync_decode_runtime_stats(&runtime_stats, &decode_state, 0.0);

    while let Ok(msg) = rx.recv() {
        match msg {
            DecodeMsg::Frame(frame) => {
                let target_time = frame.target_playout_time;
                let now_ms = crate::media::video::decode::video_decode::now_ms_f64();
                crate::xbx_log_warn!(
                    "[XbxDecodeActor] processing frame ts={} len={}",
                    frame.rtp_timestamp,
                    frame.payload.len()
                );
                decode_state.process_encoded_frame(frame, now_ms);
                if decode_state.last_decode_ok_time_ms() == Some(now_ms) {
                    recent_decode_times_ms.push_back(now_ms);
                    while let Some(front) = recent_decode_times_ms.front().copied() {
                        if now_ms - front <= 1_000.0 {
                            break;
                        }
                        recent_decode_times_ms.pop_front();
                    }
                    if let Ok(mut stats) = runtime_stats.lock() {
                        stats.latest_video_decode_ok_time_ms = Some(now_ms);
                        stats.video_decode_fps = recent_window_fps(&recent_decode_times_ms);
                    }
                }
                while let Some(render_frame) = decode_state.pop_decoded_frame(now_ms) {
                    let decoded_frame = DecodedFrame {
                        width: render_frame.width,
                        height: render_frame.height,
                        pts: target_time, // map pts back
                        surface: render_frame,
                    };

                    // DecodeActor sends decoded frame to pacer queue
                    if pacer.submit(decoded_frame).is_err() {
                        crate::xbx_log_warn!("[XbxDecodeActor] pacer queue full, drop frame");
                    }
                }
                sync_decode_runtime_stats(&runtime_stats, &decode_state, now_ms);
            }
            DecodeMsg::Flush => {
                let _ = decode_state.request_decoder_reset();
                sync_decode_runtime_stats(
                    &runtime_stats,
                    &decode_state,
                    crate::media::video::decode::video_decode::now_ms_f64(),
                );
            }
            DecodeMsg::Stop => {
                break;
            }
        }
    }
}

fn recent_window_fps(times: &std::collections::VecDeque<f64>) -> f64 {
    let len = times.len();
    if len < 2 {
        return 0.0;
    }
    let first = times.front().copied().unwrap_or_default();
    let last = times.back().copied().unwrap_or(first);
    let window_ms = (last - first).max(1.0);
    ((len.saturating_sub(1)) as f64 * 1_000.0 / window_ms).max(0.0)
}

fn sync_decode_runtime_stats(
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    decode_state: &XbxVideoDecodeState,
    now_ms: f64,
) {
    let Ok(mut stats) = runtime_stats.lock() else {
        return;
    };
    stats.video_decoder_backend_name = Some(decode_state.decoder_backend_name().to_string());
    stats.video_decoder_reset_count = decode_state.decoder_reset_count();
    stats.latest_video_decoder_reset_time_ms = decode_state.latest_decoder_reset_time_ms();
    stats.video_decode_output_drop_count_total = decode_state.decoded_frame_drop_count();
    stats.video_decoder_hardware_failure_streak = decode_state.hardware_decode_failure_streak();
    stats.latest_video_decoder_hardware_failure_time_ms =
        decode_state.latest_hardware_decode_failure_time_ms();
    stats.latest_video_decoder_hardware_failure_status =
        decode_state.latest_hardware_decode_failure_status();
    stats.video_decoder_stalled = Some(derive_decoder_stalled(&stats, now_ms));
}

fn derive_decoder_stalled(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    let packet_age_ms = stats
        .latest_video_packet_arrival_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    if packet_age_ms > DECODER_STALL_PACKET_FRESH_MAX_AGE_MS {
        return false;
    }
    let decode_age_ms = stats
        .latest_video_decode_ok_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    decode_age_ms >= DECODER_STALL_DECODE_AGE_MS
}
