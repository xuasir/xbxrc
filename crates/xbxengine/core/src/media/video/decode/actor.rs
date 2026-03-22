use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;

use crate::media::video::decode::video_decode::XbxVideoDecodeState;
use crate::media::video::pacer::actor::PacerActorHandle;
use crate::media::video::types::{DecodedFrame, EncodedFrame};
use crate::runtime_stats_sink::RuntimeStatsSink;

const DECODER_STALL_PACKET_FRESH_MAX_AGE_MS: f64 = 400.0;
const DECODER_STALL_DECODE_AGE_MS: f64 = 1_000.0;
const DECODE_MAILBOX_CAPACITY: usize = 2;

pub enum DecodeMsg {
    Frame(EncodedFrame),
    Stop,
}

pub enum DecodeSubmitError {
    Full(EncodedFrame),
    Disconnected(EncodedFrame),
}

pub struct DecodeActorHandle {
    tx: SyncSender<DecodeMsg>,
    available_slots: Arc<AtomicUsize>,
}

impl DecodeActorHandle {
    pub fn new(
        pacer: Arc<PacerActorHandle>,
        runtime_stats: Arc<std::sync::Mutex<crate::XbxEngineMediaRuntimeStats>>,
        min_delay_ms: u64,
        max_delay_ms: u64,
    ) -> Self {
        let runtime_stats = RuntimeStatsSink::new(runtime_stats);
        let (tx, rx) = mpsc::sync_channel(DECODE_MAILBOX_CAPACITY);
        let available_slots = Arc::new(AtomicUsize::new(DECODE_MAILBOX_CAPACITY));
        let available_slots_for_thread = available_slots.clone();

        thread::Builder::new()
            .name("XbxDecodeActor".into())
            .spawn(move || {
                run_decode_loop(
                    rx,
                    pacer,
                    runtime_stats,
                    min_delay_ms,
                    max_delay_ms,
                    available_slots_for_thread,
                );
            })
            .expect("Failed to spawn decode actor thread");

        Self {
            tx,
            available_slots,
        }
    }

    pub fn submit(&self, frame: EncodedFrame) -> Result<(), DecodeSubmitError> {
        match self.tx.try_send(DecodeMsg::Frame(frame)) {
            Ok(_) => {
                self.available_slots.fetch_sub(1, Ordering::AcqRel);
                Ok(())
            }
            Err(e) => match e {
                TrySendError::Full(DecodeMsg::Frame(frame)) => Err(DecodeSubmitError::Full(frame)),
                TrySendError::Disconnected(DecodeMsg::Frame(frame)) => {
                    crate::xbx_log_error!(
                        "[DecodeActorHandle] Decode thread is disconnected (likely panicked)!"
                    );
                    Err(DecodeSubmitError::Disconnected(frame))
                }
                TrySendError::Full(_) | TrySendError::Disconnected(_) => unreachable!(),
            },
        }
    }

    pub fn available_slots(&self) -> usize {
        self.available_slots.load(Ordering::Acquire)
    }

    pub fn stop(&self) {
        let _ = self.tx.send(DecodeMsg::Stop);
    }
}

fn run_decode_loop(
    rx: Receiver<DecodeMsg>,
    pacer: Arc<PacerActorHandle>,
    runtime_stats: RuntimeStatsSink,
    min_delay_ms: u64,
    max_delay_ms: u64,
    available_slots: Arc<AtomicUsize>,
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
                release_decode_slot(&available_slots);
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
                    runtime_stats.update(|stats| {
                        stats.latest_video_decode_ok_time_ms = Some(now_ms);
                        stats.video_decode_fps = recent_window_fps(&recent_decode_times_ms);
                    });
                }
                while let Some(render_frame) = decode_state.pop_decoded_frame(now_ms) {
                    let decoded_frame = DecodedFrame {
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
            DecodeMsg::Stop => {
                break;
            }
        }
    }
}

fn release_decode_slot(available_slots: &Arc<AtomicUsize>) {
    let _ = available_slots.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some(current.saturating_add(1).min(DECODE_MAILBOX_CAPACITY))
    });
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
    runtime_stats: &RuntimeStatsSink,
    decode_state: &XbxVideoDecodeState,
    now_ms: f64,
) {
    let video_decoder_stalled = derive_decoder_stalled(runtime_stats, now_ms);
    runtime_stats.update(|stats| {
        stats.video_decoder_backend_name = Some(decode_state.decoder_backend_name().to_string());
        stats.video_decoder_reset_count = decode_state.decoder_reset_count();
        stats.latest_video_decoder_reset_time_ms = decode_state.latest_decoder_reset_time_ms();
        stats.video_decode_output_drop_count_total = decode_state.decoded_frame_drop_count();
        stats.video_decoder_hardware_failure_streak = decode_state.hardware_decode_failure_streak();
        stats.latest_video_decoder_hardware_failure_time_ms =
            decode_state.latest_hardware_decode_failure_time_ms();
        stats.latest_video_decoder_hardware_failure_status =
            decode_state.latest_hardware_decode_failure_status();
        stats.video_decoder_stalled = Some(video_decoder_stalled);
    });
}

fn derive_decoder_stalled(runtime_stats: &RuntimeStatsSink, now_ms: f64) -> bool {
    runtime_stats
        .read(|stats| {
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
        })
        .unwrap_or(false)
}
