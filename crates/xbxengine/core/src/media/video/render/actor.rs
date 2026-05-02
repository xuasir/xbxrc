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
    let mut render_mailbox_decision_id = 0u64;

    while let Ok(msg) = rx.recv() {
        match msg {
            RendererMsg::Frame(frame) => {
                runtime_stats.update(|stats| {
                    stats.video_renderer_submit_count_total =
                        stats.video_renderer_submit_count_total.saturating_add(1);
                });
                let flow_context = read_renderer_flow_context(&runtime_stats);
                let flow_frame = frame.clone();
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
                let frame_rtp_timestamp = frame.surface.rtp_timestamp;
                let frame_is_keyframe = frame.surface.is_keyframe;
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
                        log_renderer_flow(
                            "submit",
                            &flow_frame,
                            &flow_context,
                            None,
                            None,
                            outcome.overwritten_pending_frame,
                        );
                        runtime_stats.update(|stats| {
                            stats.latest_observation_label =
                                Some("rendererFrameAccepted".to_string());
                            stats.latest_observation_summary = Some(format!(
                                "frameSeq={} rtpTimestamp={} isKeyframe={} overwrittenPendingFrame={}",
                                present_frame_seq,
                                frame_rtp_timestamp
                                    .map(|value| value.to_string())
                                    .unwrap_or_else(|| "none".to_string()),
                                frame_is_keyframe,
                                outcome.overwritten_pending_frame,
                            ));
                        });
                        if outcome.overwritten_pending_frame {
                            log_renderer_flow(
                                "mailboxOverwrite",
                                &flow_frame,
                                &flow_context,
                                Some("mailboxOverwrite"),
                                outcome.overwritten_frame_seq,
                                true,
                            );
                            record_pipeline_frame_drop(
                                &runtime_stats,
                                &mut frame_drop_observation_id,
                                "render",
                                "replace",
                                Some("mailboxOverwrite"),
                                present_observed_at_ms,
                                outcome.overwritten_frame_width.unwrap_or(present_width),
                                outcome.overwritten_frame_height.unwrap_or(present_height),
                                false,
                                1,
                                None,
                                outcome.overwritten_frame_seq,
                                None,
                                None,
                                state
                                    .latest_render_mailbox_decision()
                                    .and_then(|decision| decision.replacement_decision.clone()),
                            );
                        }
                        if let Some(decision) = state.latest_render_mailbox_decision() {
                            if decision.detail == "mailboxRecovered"
                                && decision.frame_seq == Some(present_frame_seq)
                            {
                                log_renderer_flow(
                                    "mailboxRecovered",
                                    &flow_frame,
                                    &flow_context,
                                    Some("mailboxRecovered"),
                                    None,
                                    false,
                                );
                            }
                        }
                    }
                    Err(e) => {
                        log_renderer_flow(
                            "presentFailed",
                            &flow_frame,
                            &flow_context,
                            Some("presentError"),
                            None,
                            false,
                        );
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
                            None,
                        );
                        crate::xbx_log_error!("[XbxRendererActor] present_frame error: {:?}", e);
                    }
                }
                if let Some(decision) = state.latest_render_mailbox_decision() {
                    if decision.decision_id != render_mailbox_decision_id {
                        render_mailbox_decision_id = decision.decision_id;
                        runtime_stats.update(|stats| {
                            stats.latest_render_mailbox_decision = Some(
                                crate::api::backend::XbxEnginePipelineCandidateDecisionObservation {
                                    decision_id: decision.decision_id,
                                    state: decision.state.as_str().to_string(),
                                    action: decision.action.to_string(),
                                detail: decision.detail.to_string(),
                                frame_seq: decision.frame_seq,
                                replacement_decision: decision.replacement_decision.clone(),
                                observed_at_ms: decision.observed_at_ms,
                            },
                        );
                            stats.latest_observation_label = Some("renderMailboxState".to_string());
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

#[derive(Clone, Copy, Debug, Default)]
struct RendererFlowContext {
    host_tick_epoch: u64,
    present_epoch: u64,
}

fn read_renderer_flow_context(runtime_stats: &RuntimeStatsSink) -> RendererFlowContext {
    runtime_stats
        .read(|stats| RendererFlowContext {
            host_tick_epoch: stats.host_display_tick_epoch,
            present_epoch: stats.host_frame_present_epoch,
        })
        .unwrap_or_default()
}

fn log_renderer_flow(
    event: &str,
    frame: &DecodedFrame,
    flow_context: &RendererFlowContext,
    reason: Option<&str>,
    related_frame_seq: Option<u64>,
    overwritten_pending_frame: bool,
) {
    crate::xbx_log_warn!(
        "[playback-flow][renderer] event={} reason={} frameSeq={} rtpTimestamp={} isKeyframe={} observedAtMs={} overwrittenPendingFrame={} overwrittenFrameSeq={} hostTickEpoch={} hostFramePresentEpoch={}",
        event,
        reason.unwrap_or("-"),
        frame.surface.frame_seq,
        frame.surface
            .rtp_timestamp
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        frame.surface.is_keyframe,
        frame.surface.rendered_at_ms,
        overwritten_pending_frame,
        related_frame_seq
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        flow_context.host_tick_epoch,
        flow_context.present_epoch,
    );
}

#[cfg(test)]
mod tests {
    use super::{run_renderer_loop, RendererMsg};
    use crate::api::backend::XbxEngineMediaRuntimeStats;
    use crate::media::video::render::renderer::{XbxRenderFrame, XbxRenderState};
    use crate::media::video::types::{DecodedFrame, FrameRecoveryDisposition};
    use crate::runtime_stats_sink::RuntimeStatsSink;
    use crate::XbxEngineRenderPixelData;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    fn make_decoded_frame(frame_seq: u64, width: u32, height: u32) -> DecodedFrame {
        let bytes = vec![frame_seq as u8; (width * height * 4) as usize];
        DecodedFrame {
            pts: Instant::now(),
            rtp_timestamp: frame_seq as u32,
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: frame_seq == 1,
            clean_anchor_commit_recovery_epoch: None,
            presentation_value_role: None,
            budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
            frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            surface: XbxRenderFrame {
                width,
                height,
                frame_seq,
                rendered_at_ms: frame_seq as f64,
                rtp_timestamp: Some(frame_seq as u32),
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: frame_seq == 1,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from(bytes),
                },
            },
        }
    }

    fn make_decoded_frame_with_epoch(
        frame_seq: u64,
        width: u32,
        height: u32,
        recovery_epoch_tag: Option<u64>,
    ) -> DecodedFrame {
        let mut frame = make_decoded_frame(frame_seq, width, height);
        frame.recovery_epoch_tag = recovery_epoch_tag;
        frame.surface.recovery_epoch_tag = recovery_epoch_tag;
        frame
    }

    #[test]
    fn renderer_actor_projects_latest_overwrite_decision_into_runtime_stats() {
        let (tx, rx) = mpsc::sync_channel(4);
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let runtime_stats_sink = RuntimeStatsSink::new(runtime_stats.clone());

        tx.send(RendererMsg::Frame(make_decoded_frame(2, 2, 2)))
            .expect("first frame");
        tx.send(RendererMsg::Frame(make_decoded_frame(3, 2, 2)))
            .expect("second frame");
        tx.send(RendererMsg::Stop).expect("stop");

        run_renderer_loop(rx, render_state.clone(), runtime_stats_sink);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_renderer_submit_count_total, 2);
        assert!(
            matches!(
                stats.latest_observation_label.as_deref(),
                Some("renderMailboxState" | "rendererFrameAccepted")
            ),
            "unexpected latest observation label: {:?}",
            stats.latest_observation_label
        );
        let summary = stats
            .latest_observation_summary
            .clone()
            .expect("render summary");
        assert!(
            summary.contains("latest-overwrite:replace:mailboxOverwrite:seq=1")
                || summary.contains("frameSeq=2")
                || summary.contains("latest-overwrite:replace:mailboxOverwrite:seq=2")
                || summary.contains("frameSeq=3"),
            "unexpected render summary: {summary}"
        );
        let decision = stats
            .latest_render_mailbox_decision
            .clone()
            .expect("latest render decision");
        assert_eq!(decision.state, "latest-overwrite");
        assert_eq!(decision.action, "replace");
        assert_eq!(decision.detail, "mailboxOverwrite");
        assert_eq!(decision.frame_seq, Some(2));
    }

    #[test]
    fn renderer_actor_counts_present_error_as_drop() {
        let (tx, rx) = mpsc::sync_channel(2);
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let runtime_stats_sink = RuntimeStatsSink::new(runtime_stats.clone());

        let mut bad_frame = make_decoded_frame(7, 2, 2);
        bad_frame.surface.pixel_data = XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([7u8; 4]),
        };

        tx.send(RendererMsg::Frame(bad_frame)).expect("bad frame");
        tx.send(RendererMsg::Stop).expect("stop");
        run_renderer_loop(rx, render_state, runtime_stats_sink);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_renderer_submit_count_total, 1);
        assert_eq!(stats.video_renderer_drop_count_total, 1);
    }

    #[test]
    fn renderer_actor_overwrites_pending_frame_without_rejecting_lower_epoch_submit() {
        let (tx, rx) = mpsc::sync_channel(3);
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let runtime_stats_sink = RuntimeStatsSink::new(runtime_stats.clone());

        tx.send(RendererMsg::Frame(make_decoded_frame_with_epoch(
            200,
            2,
            2,
            Some(4),
        )))
        .expect("higher epoch frame");
        tx.send(RendererMsg::Frame(make_decoded_frame_with_epoch(
            150,
            2,
            2,
            Some(3),
        )))
        .expect("lower epoch frame");
        tx.send(RendererMsg::Stop).expect("stop");

        run_renderer_loop(rx, render_state.clone(), runtime_stats_sink);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_renderer_submit_count_total, 2);
        assert_eq!(stats.video_renderer_drop_count_total, 0);
        let decision = stats
            .latest_render_mailbox_decision
            .as_ref()
            .expect("latest render decision");
        assert_eq!(decision.state, "latest-overwrite");
        assert_eq!(decision.action, "replace");
        assert_eq!(decision.detail, "mailboxOverwrite");
        assert_eq!(decision.frame_seq, Some(200));
    }
}
