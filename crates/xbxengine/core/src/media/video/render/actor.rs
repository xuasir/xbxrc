use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::media::video::render::renderer::XbxRenderState;
use crate::media::video::types::DecodedFrame;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::pipeline::observation::record_pipeline_frame_drop;

pub enum RendererMsg {
    Frame(DecodedFrame),
    Stop,
}

pub struct RendererActorHandle {
    tx: SyncSender<RendererMsg>,
}

impl RendererActorHandle {
    pub fn new(
        render_state: Arc<Mutex<XbxRenderState>>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Self {
        let runtime_stats = RuntimeStatsSink::new(runtime_stats);
        let (tx, rx) = mpsc::sync_channel(1);

        thread::Builder::new()
            .name("XbxRendererActor".into())
            .spawn(move || {
                run_renderer_loop(rx, render_state, runtime_stats);
            })
            .expect("Failed to spawn renderer actor thread");

        Self { tx }
    }

    pub fn submit(&self, frame: DecodedFrame) -> Result<(), TrySendError<RendererMsg>> {
        self.tx.try_send(RendererMsg::Frame(frame))
    }

    pub fn stop(&self) {
        let _ = self.tx.send(RendererMsg::Stop);
    }
}

fn run_renderer_loop(
    rx: Receiver<RendererMsg>,
    render_state: Arc<Mutex<XbxRenderState>>,
    runtime_stats: RuntimeStatsSink,
) {
    let mut frame_drop_observation_id = 0u64;
    let mut render_candidate_decision_id = 0u64;

    while let Ok(msg) = rx.recv() {
        match msg {
            RendererMsg::Frame(frame) => {
                runtime_stats.update(|stats| {
                    stats.video_renderer_submit_count_total =
                        stats.video_renderer_submit_count_total.saturating_add(1);
                });
                let mut state = match render_state.lock() {
                    Ok(guard) => guard,
                    Err(_) => {
                        runtime_stats.update(|stats| {
                            stats.video_renderer_drop_count_total =
                                stats.video_renderer_drop_count_total.saturating_add(1);
                        });
                        continue;
                    }
                };

                // Set the current real-time ms before presenting so metrics are correct
                let mut render_frame = frame.surface;
                render_frame.rendered_at_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as f64)
                    .unwrap_or(0.0);

                let present_width = render_frame.width;
                let present_height = render_frame.height;
                let present_frame_seq = render_frame.frame_seq;
                let present_observed_at_ms = render_frame.rendered_at_ms;

                match state.present_frame(render_frame) {
                    Ok((_frame_stats, outcome)) => {
                        if outcome.overwritten_previous_latest {
                            record_pipeline_frame_drop(
                                &runtime_stats,
                                &mut frame_drop_observation_id,
                                "render",
                                "replace",
                                Some("latestSlotOverwrite"),
                                present_observed_at_ms,
                                outcome.overwritten_frame_width.unwrap_or(present_width),
                                outcome.overwritten_frame_height.unwrap_or(present_height),
                                false,
                                1,
                                None,
                                outcome.overwritten_frame_seq,
                                None,
                                None,
                            );
                        }
                    }
                    Err(e) => {
                        runtime_stats.update(|stats| {
                            stats.video_renderer_drop_count_total =
                                stats.video_renderer_drop_count_total.saturating_add(1);
                        });
                        record_pipeline_frame_drop(
                            &runtime_stats,
                            &mut frame_drop_observation_id,
                            "render",
                            "drop",
                            Some("presentError"),
                            present_observed_at_ms,
                            present_width,
                            present_height,
                            false,
                            1,
                            None,
                            Some(present_frame_seq),
                            None,
                            None,
                        );
                        crate::xbx_log_error!("[XbxRendererActor] present_frame error: {:?}", e);
                    }
                }
                if let Some(decision) = state.latest_render_candidate_decision() {
                    if decision.decision_id != render_candidate_decision_id {
                        render_candidate_decision_id = decision.decision_id;
                        runtime_stats.update(|stats| {
                            stats.latest_render_candidate_decision = Some(
                                crate::api::backend::XbxEnginePipelineCandidateDecisionObservation {
                                    decision_id: decision.decision_id,
                                    state: decision.state.as_str().to_string(),
                                    action: decision.action.to_string(),
                                    detail: decision.detail.to_string(),
                                    frame_seq: decision.frame_seq,
                                    observed_at_ms: decision.observed_at_ms,
                                },
                            );
                            stats.latest_observation_label =
                                Some("renderCandidateState".to_string());
                            stats.latest_observation_summary = Some(format!(
                                "{}:{}:{}:seq={}",
                                decision.state.as_str(),
                                decision.action,
                                decision.detail,
                                decision
                                    .frame_seq
                                    .map(|seq| seq.to_string())
                                    .unwrap_or_else(|| "-".to_string())
                            ));
                        });
                    }
                }
            }
            RendererMsg::Stop => {
                break;
            }
        }
    }
}
