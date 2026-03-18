use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::media::video::render::actor::RendererActorHandle;
use crate::media::video::render::pacer::{FramePacingAction, FramePacingPolicy};
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
    let fallback_refresh_interval_ms = refresh_interval_ms;
    let mut catch_up_mode = false;

    while let Ok(msg) = rx.recv() {
        match msg {
            PacerMsg::Frame(frame) => {
                if let Ok(mut stats) = runtime_stats.lock() {
                    stats.video_pacer_submit_count_total =
                        stats.video_pacer_submit_count_total.saturating_add(1);
                }
                let (effective_refresh_interval_ms, host_frame_age_budget_ms) =
                    resolve_host_pacing_timing(
                        runtime_stats.as_ref(),
                        fallback_refresh_interval_ms,
                    );
                let pacing_policy = FramePacingPolicy::with_dynamic_budget(
                    effective_refresh_interval_ms,
                    host_frame_age_budget_ms.map(|budget_ms| budget_ms.round() as u64),
                );
                let decision = pacing_policy.decide(Instant::now(), frame.pts, catch_up_mode);
                if decision.enter_catch_up_mode {
                    catch_up_mode = true;
                }
                if decision.exit_catch_up_mode {
                    catch_up_mode = false;
                }
                match decision.action {
                    FramePacingAction::Drop => {
                        if let Ok(mut stats) = runtime_stats.lock() {
                            stats.video_pacer_drop_count_total =
                                stats.video_pacer_drop_count_total.saturating_add(1);
                        }
                        continue;
                    }
                    FramePacingAction::SubmitNow => {}
                    FramePacingAction::Sleep(duration) => {
                        thread::sleep(duration);
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

fn resolve_host_pacing_timing(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    fallback_refresh_interval_ms: u64,
) -> (u64, Option<f64>) {
    let Ok(stats) = runtime_stats.lock() else {
        return (fallback_refresh_interval_ms, None);
    };
    let refresh_interval_ms = stats
        .host_display_interval_ms
        .map(|interval_ms| interval_ms.round() as u64)
        .filter(|interval_ms| *interval_ms > 0)
        .unwrap_or(fallback_refresh_interval_ms);
    (refresh_interval_ms, stats.host_frame_age_budget_ms)
}
