use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use webrtc::{peer_connection::RTCPeerConnection, track::track_remote::TrackRemote};

use crate::{
    runtime_stats_sink::RuntimeStatsSink, transport::webrtc::startup_recovery::SessionPhase,
    XbxEngineMediaRuntimeStats, XbxEngineVideoBweObservation, XbxEngineVideoTrackStatus,
    XbxEngineWebRtcRuntimeConfig,
};
use xbxengine_protocol::XbxEngineTransportStateDto;

use super::{
    now_ms_f64, video_track_bwe_evaluator::VideoTrackBweEvaluatorState,
    video_track_observation_collector::VideoTrackObservationCollectorState,
};

pub(super) fn spawn_video_track_stats_loop(
    stats_track: Arc<TrackRemote>,
    peer_connection: Arc<RTCPeerConnection>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    webrtc_config: XbxEngineWebRtcRuntimeConfig,
    task_generation: Arc<AtomicU64>,
    current_generation: u64,
    video_mime_type: Option<String>,
) {
    tokio::spawn(async move {
        let base_feedback_interval = std::time::Duration::from_millis(
            webrtc_config.video_pipeline.feedback_interval_ms.max(50),
        );
        let pressure_feedback_interval = std::time::Duration::from_millis(
            (webrtc_config.video_pipeline.feedback_interval_ms / 2).clamp(
                40,
                webrtc_config.video_pipeline.feedback_interval_ms.max(50),
            ),
        );
        let runtime_stats = RuntimeStatsSink::new(runtime_stats);
        let bwe_stream_started_at = std::time::Instant::now();
        let bwe_startup_grace =
            std::time::Duration::from_millis(webrtc_config.recovery.first_frame_grace_ms);
        let mut collector_state = VideoTrackObservationCollectorState::new(now_ms_f64());
        let mut bwe_evaluator_state = VideoTrackBweEvaluatorState::new(
            webrtc_config
                .forced_remb_kbps
                .unwrap_or(webrtc_config.remb_floor_kbps),
        );

        loop {
            let next_interval = select_feedback_interval(
                &runtime_stats,
                base_feedback_interval,
                pressure_feedback_interval,
            );
            tokio::time::sleep(next_interval).await;
            if task_generation.load(Ordering::SeqCst) != current_generation {
                break;
            }

            let Some(observation) = collector_state
                .collect(&stats_track, &peer_connection, &runtime_stats)
                .await
            else {
                continue;
            };
            let bwe_evaluation = bwe_evaluator_state.evaluate(
                &runtime_stats,
                &webrtc_config,
                &observation,
                bwe_stream_started_at,
                bwe_startup_grace,
            );
            let observed_at_ms = now_ms_f64();

            runtime_stats.record_transport_metrics(
                observation.rtt_ms,
                observation.rtt_source.clone(),
                observation.fraction_lost,
                observation
                    .synthetic_loss_ratio
                    .max(observation.fraction_lost),
                observation.transport_path.clone(),
                observation.actual_kbps,
                observation.current_bytes,
            );

            runtime_stats.update(|shared| {
                // transport facts 走 observation bus；这里只保留策略投影与 BWE 结果。
                shared.video_remb_bps = Some(bwe_evaluation.target_remb_kbps.saturating_mul(1000));
                shared.session_phase = Some(bwe_evaluation.session_phase.as_str().to_string());
                shared.transport_policy_profile =
                    Some(bwe_evaluation.transport_policy_profile.clone());
                if matches!(bwe_evaluation.session_phase, SessionPhase::Steady) {
                    shared.recovery_diagnosis = None;
                }
                shared.recovery_coupling_mode = Some(bwe_evaluation.recovery_coupling_mode.clone());
                shared.recovery_coupling_summary =
                    Some(bwe_evaluation.recovery_coupling_summary.clone());
                shared.direct_gaming_bitrate_band =
                    bwe_evaluation.direct_gaming_bitrate_band.clone();
                if shared.latest_video_track_status.is_none()
                    && shared.inbound_audio_bytes_total > 0
                    && shared.inbound_video_bytes_total == 0
                {
                    shared.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
                        state: "audioOnly".to_string(),
                        video_width: None,
                        video_height: None,
                        mime_type: None,
                        transport_state: shared.transport_state.clone(),
                        video_bytes_total: 0,
                        video_packet_count_total: 0,
                        audio_bytes_total: shared.inbound_audio_bytes_total,
                        observed_at_ms,
                    });
                }
                shared.latest_video_bwe_observation = Some(XbxEngineVideoBweObservation {
                    observation_id: bwe_evaluation.observation_id,
                    mode: webrtc_config.bwe_mode.clone(),
                    decision_reason: bwe_evaluation.decision_reason.clone(),
                    target_remb_kbps: bwe_evaluation.target_remb_kbps,
                    observed_remb_kbps: observation.observed_remb_kbps,
                    actual_video_bitrate_kbps: observation.actual_kbps,
                    loss_ratio: observation.fraction_lost,
                    rtt_ms: observation.rtt_ms,
                    transport_path: observation.transport_path.clone(),
                    twcc_feedback_interval_ms: bwe_evaluation.twcc_feedback_interval_ms,
                    twcc_observed_packet_count: bwe_evaluation.twcc_observed_packet_count,
                    twcc_covered_sequence_span: bwe_evaluation.twcc_covered_sequence_span,
                    twcc_receive_bitrate_kbps: bwe_evaluation.twcc_receive_bitrate_kbps,
                    twcc_delivery_ratio: bwe_evaluation.twcc_delivery_ratio,
                    twcc_loss_ratio: bwe_evaluation.twcc_loss_ratio,
                    observed_at_ms,
                });
            });

            if observation.should_mark_video_started {
                let audio_bytes_total = runtime_stats
                    .read(|stats| stats.inbound_audio_bytes_total)
                    .unwrap_or(0);
                update_video_track_status(
                    &runtime_stats,
                    XbxEngineVideoTrackStatus {
                        state: "videoRtpStarted".to_string(),
                        video_width: None,
                        video_height: None,
                        mime_type: video_mime_type.clone(),
                        transport_state: XbxEngineTransportStateDto::Connected,
                        video_bytes_total: observation.current_bytes,
                        video_packet_count_total: observation.packets_received,
                        audio_bytes_total,
                        observed_at_ms: observation.sampled_at_ms,
                    },
                );
            }

            use webrtc::rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::*;
            let remb = ReceiverEstimatedMaximumBitrate {
                bitrate: (bwe_evaluation.target_remb_kbps as f32) * 1000.0,
                ssrcs: vec![stats_track.ssrc()],
                ..Default::default()
            };
            let inject_result = peer_connection.write_rtcp(&[Box::new(remb)]).await;

            if let Err(error) = inject_result {
                crate::xbx_log_warn!("[xbxengine][BWE] REMB injection failed: {:?}", error);
            }
        }
    });
}

fn update_video_track_status(runtime_stats: &RuntimeStatsSink, status: XbxEngineVideoTrackStatus) {
    let should_publish = runtime_stats
        .read(|shared| shared.latest_video_track_status.as_ref() != Some(&status))
        .unwrap_or(true);
    if should_publish {
        runtime_stats.record_video_track_status(status);
    }
}

fn select_feedback_interval(
    runtime_stats: &RuntimeStatsSink,
    base_feedback_interval: std::time::Duration,
    pressure_feedback_interval: std::time::Duration,
) -> std::time::Duration {
    let pressure_active = runtime_stats
        .read(|stats| {
            let now_ms = now_ms_f64();
            let recent_transport_failure = stats
                .latest_video_nack_observation
                .as_ref()
                .is_some_and(|nack| {
                    nack.observed_at_ms > 0.0
                        && (now_ms - nack.observed_at_ms).max(0.0) <= 320.0
                        && (nack.action.starts_with("expired")
                            || nack.action == "recoveredLate"
                            || nack.source == "sampleLoss")
                })
                || stats
                    .latest_video_packet_gap
                    .as_ref()
                    .is_some_and(|gap| (now_ms - gap.observed_at_ms).max(0.0) <= 220.0);
            stats.video_pending_missing_packets > 0
                || stats.recovery_coupling_mode.as_deref() == Some("waitingKeyframe")
                || recent_transport_failure
        })
        .unwrap_or(false);
    if pressure_active {
        pressure_feedback_interval
    } else {
        base_feedback_interval
    }
}
