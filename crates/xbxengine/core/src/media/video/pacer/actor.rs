use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::media::video::render::actor::RendererActorHandle;
use crate::media::video::types::DecodedFrame;

pub enum PacerMsg {
    Frame(DecodedFrame),
    Flush,
    Stop,
}

pub struct PacerActorHandle {
    tx: SyncSender<PacerMsg>,
}

impl PacerActorHandle {
    pub fn new(
        renderer: Arc<RendererActorHandle>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        refresh_interval_ms: u64,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel(2);

        thread::Builder::new()
            .name("XbxPacerActor".into())
            .spawn(move || {
                run_pacer_loop(rx, renderer, runtime_stats, refresh_interval_ms);
            })
            .expect("Failed to spawn pacer actor thread");

        Self { tx }
    }

    pub fn submit(&self, frame: DecodedFrame) -> Result<(), TrySendError<PacerMsg>> {
        self.tx.try_send(PacerMsg::Frame(frame))
    }

    pub fn flush(&self) {
        let _ = self.tx.send(PacerMsg::Flush);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(PacerMsg::Stop);
    }
}

fn run_pacer_loop(
    rx: Receiver<PacerMsg>,
    renderer: Arc<RendererActorHandle>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    refresh_interval_ms: u64,
) {
    let max_sleep = Duration::from_millis(refresh_interval_ms.max(1));
    let catch_up_threshold = Duration::from_millis(500);
    let mut catch_up_mode = false;

    while let Ok(msg) = rx.recv() {
        match msg {
            PacerMsg::Frame(frame) => {
                if let Ok(mut stats) = runtime_stats.lock() {
                    stats.video_pacer_submit_count_total =
                        stats.video_pacer_submit_count_total.saturating_add(1);
                }
                let now = Instant::now();
                let deadline = frame.pts;

                if catch_up_mode {
                    if now > deadline + catch_up_threshold {
                        if let Ok(mut stats) = runtime_stats.lock() {
                            stats.video_pacer_drop_count_total =
                                stats.video_pacer_drop_count_total.saturating_add(1);
                        }
                        continue;
                    } else {
                        catch_up_mode = false;
                    }
                }

                // If massive backlog, enter catch_up_mode
                if now > deadline + catch_up_threshold {
                    catch_up_mode = true;
                    if let Ok(mut stats) = runtime_stats.lock() {
                        stats.video_pacer_drop_count_total =
                            stats.video_pacer_drop_count_total.saturating_add(1);
                    }
                    continue;
                }

                // If late, but within 500ms, just submit immediately (don't sleep)
                if now >= deadline {
                    // Late or Perfect time.
                    // (removed strict drop if now > deadline + refresh_interval)
                } else {
                    // Early frame
                    let sleep_time = deadline.duration_since(now);
                    if sleep_time <= max_sleep {
                        thread::sleep(sleep_time);
                    } else {
                        // 保护：playout 目标异常偏大时不阻塞渲染节奏，优先快速出帧。
                        crate::xbx_log_debug!(
                            "[XbxPacerActor] skip long sleep: {:.2}ms",
                            sleep_time.as_millis()
                        );
                    }
                }

                // Render queue size 1 as per RFC (or renderer limits itself).
                // Renderer handles immediate flip.
                if renderer.submit(frame).is_err() {
                    if let Ok(mut stats) = runtime_stats.lock() {
                        stats.video_pacer_drop_count_total =
                            stats.video_pacer_drop_count_total.saturating_add(1);
                    }
                    crate::xbx_log_warn!("[XbxPacerActor] renderer queue full, frame dropped!");
                }
            }
            PacerMsg::Flush => {
                catch_up_mode = false;
            }
            PacerMsg::Stop => {
                break;
            }
        }
    }
}
