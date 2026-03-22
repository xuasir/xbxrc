use std::sync::{Arc, Mutex};

use crate::{
    XbxEngineMediaRuntimeStats, XbxEngineVideoFrameDropObservation, XbxEngineVideoNackObservation,
    XbxEngineVideoPacketGapObservation, XbxEngineVideoRtxReinjectObservation,
};

#[derive(Clone)]
pub(crate) struct ObservationBus {
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
}

#[derive(Clone, Debug)]
pub(crate) enum ObservationEvent {
    FrameArrival {
        now_ms: f64,
        frame_count: u64,
        fps: f64,
    },
    StreamDimensions {
        width: u32,
        height: u32,
    },
    VideoRtxReinject {
        observation: XbxEngineVideoRtxReinjectObservation,
    },
    HostVideoTiming {
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    },
    TransportMetrics {
        video_rtt_ms: Option<f64>,
        video_rtt_source: Option<String>,
        inbound_video_loss_ratio_5s: f64,
        inbound_video_loss_ratio_1s: f64,
        transport_path: Option<String>,
        inbound_video_bitrate_kbps: f64,
        inbound_primary_video_bytes_total: u64,
    },
    VideoFrameDrop {
        observation: XbxEngineVideoFrameDropObservation,
    },
    InboundVideoPacketLossEstimate {
        packet_count: u16,
    },
    VideoLossFinalized {
        packet_count: usize,
    },
    VideoPendingMissingPackets {
        pending_count: usize,
    },
    NackSent {
        batch_len: usize,
        pending_count: usize,
    },
    LatestVideoNackObservation {
        observation: XbxEngineVideoNackObservation,
    },
    NackRecovered {
        was_late: bool,
        recovery_time_ms: f64,
        pending_count: usize,
        observation: XbxEngineVideoNackObservation,
    },
    LatestVideoPacketGap {
        observation: XbxEngineVideoPacketGapObservation,
        latest_sequence: u16,
    },
}

#[derive(Clone, Debug)]
struct ObservationPublication {
    label: String,
    summary: String,
}

impl ObservationBus {
    pub(crate) fn new(runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>) -> Self {
        Self { runtime_stats }
    }

    pub(crate) fn update(&self, apply: impl FnOnce(&mut XbxEngineMediaRuntimeStats)) {
        if let Ok(mut stats) = self.runtime_stats.lock() {
            apply(&mut stats);
        }
    }

    pub(crate) fn read<T>(
        &self,
        project: impl FnOnce(&XbxEngineMediaRuntimeStats) -> T,
    ) -> Option<T> {
        self.runtime_stats.lock().ok().map(|stats| project(&stats))
    }

    pub(crate) fn publish(&self, event: ObservationEvent) {
        self.dispatch(event);
    }

    fn dispatch(&self, event: ObservationEvent) {
        let publication = summarize_event(&event);
        self.update(|stats| {
            stats.latest_observation_label = Some(publication.label);
            stats.latest_observation_summary = Some(publication.summary);
            apply_event(stats, event);
        });
    }
}

fn summarize_event(event: &ObservationEvent) -> ObservationPublication {
    match event {
        ObservationEvent::FrameArrival {
            frame_count,
            fps,
            ..
        } => ObservationPublication {
            label: "frameArrival".to_string(),
            summary: format!("count={frame_count} fps={fps:.1}"),
        },
        ObservationEvent::StreamDimensions { width, height } => ObservationPublication {
            label: "streamDimensions".to_string(),
            summary: format!("{width}x{height}"),
        },
        ObservationEvent::VideoRtxReinject { observation } => ObservationPublication {
            label: "rtxReinject".to_string(),
            summary: format!(
                "stage={} seq={} native={:?} pending={} headMatch={} rangeMatch={} gap={:?} nack={:?}..{:?}",
                observation.stage,
                observation.sequence_number,
                observation.native_sequence_number,
                observation.pending_queue_len,
                observation.matched_head_gap,
                observation.matched_nack_range,
                observation.matched_gap_sequence,
                observation.matched_nack_first_sequence,
                observation.matched_nack_last_sequence
            ),
        },
        ObservationEvent::HostVideoTiming {
            host_display_interval_ms,
            host_frame_age_budget_ms,
        } => ObservationPublication {
            label: "hostVideoTiming".to_string(),
            summary: format!(
                "display={:?} frameAgeBudget={:?}",
                host_display_interval_ms, host_frame_age_budget_ms
            ),
        },
        ObservationEvent::TransportMetrics {
            video_rtt_ms,
            inbound_video_loss_ratio_5s,
            inbound_video_loss_ratio_1s,
            transport_path,
            inbound_video_bitrate_kbps,
            ..
        } => ObservationPublication {
            label: "transportMetrics".to_string(),
            summary: format!(
                "rtt={video_rtt_ms:?} path={} loss5={inbound_video_loss_ratio_5s:.3} loss1={inbound_video_loss_ratio_1s:.3} kbps={inbound_video_bitrate_kbps:.1}",
                transport_path.as_deref().unwrap_or("-"),
            ),
        },
        ObservationEvent::VideoFrameDrop { observation } => ObservationPublication {
            label: "videoFrameDrop".to_string(),
            summary: format!(
                "reason={} q={} keyframe={} size={}x{}",
                observation.reason,
                observation.queue_depth,
                observation.is_keyframe,
                observation.width,
                observation.height
            ),
        },
        ObservationEvent::InboundVideoPacketLossEstimate { packet_count } => ObservationPublication {
            label: "packetLossEstimate".to_string(),
            summary: format!("+{packet_count}"),
        },
        ObservationEvent::VideoLossFinalized { packet_count } => ObservationPublication {
            label: "videoLossFinalized".to_string(),
            summary: format!("+{packet_count}"),
        },
        ObservationEvent::VideoPendingMissingPackets { pending_count } => ObservationPublication {
            label: "pendingMissingPackets".to_string(),
            summary: format!("count={pending_count}"),
        },
        ObservationEvent::NackSent {
            batch_len,
            pending_count,
        } => ObservationPublication {
            label: "nackSent".to_string(),
            summary: format!("batch={batch_len} pending={pending_count}"),
        },
        ObservationEvent::LatestVideoNackObservation { observation } => ObservationPublication {
            label: "nackObservation".to_string(),
            summary: format!(
                "action={} seq={}..{} retries={} source={}",
                observation.action,
                observation.first_sequence,
                observation.last_sequence,
                observation.retry_count,
                observation.source
            ),
        },
        ObservationEvent::NackRecovered {
            was_late,
            recovery_time_ms,
            pending_count,
            observation,
        } => ObservationPublication {
            label: "nackRecovered".to_string(),
            summary: format!(
                "late={} rtt={recovery_time_ms:.1}ms pending={} seq={} source={}",
                was_late,
                pending_count,
                observation.first_sequence,
                observation.source
            ),
        },
        ObservationEvent::LatestVideoPacketGap {
            observation,
            latest_sequence,
        } => ObservationPublication {
            label: "packetGap".to_string(),
            summary: format!(
                "source={} missing={} seq={}..{}",
                observation.source, observation.missing_count, observation.expected_sequence, latest_sequence
            ),
        },
    }
}

fn apply_event(stats: &mut XbxEngineMediaRuntimeStats, event: ObservationEvent) {
    match event {
        ObservationEvent::FrameArrival {
            now_ms,
            frame_count,
            fps,
        } => {
            stats.latest_video_packet_arrival_time_ms = Some(now_ms);
            stats.inbound_video_packet_count_total = frame_count;
            stats.inbound_video_frame_rate_fps = fps;
        }
        ObservationEvent::StreamDimensions { width, height } => {
            if width > 0 {
                stats.latest_video_stream_width = Some(width);
                stats.latest_video_stream_height = Some(height);
            }
        }
        ObservationEvent::VideoRtxReinject { observation } => {
            if observation.stage == "queued" {
                if observation.matched_head_gap {
                    stats.video_rtx_reinject_head_match_count_total = stats
                        .video_rtx_reinject_head_match_count_total
                        .saturating_add(1);
                } else if observation.matched_nack_range {
                    stats.video_rtx_reinject_range_match_count_total = stats
                        .video_rtx_reinject_range_match_count_total
                        .saturating_add(1);
                } else {
                    stats.video_rtx_reinject_miss_count_total =
                        stats.video_rtx_reinject_miss_count_total.saturating_add(1);
                }
            }
            stats.latest_video_rtx_reinject_observation = Some(observation);
        }
        ObservationEvent::HostVideoTiming {
            host_display_interval_ms,
            host_frame_age_budget_ms,
        } => {
            stats.host_display_interval_ms = host_display_interval_ms;
            stats.host_frame_age_budget_ms = host_frame_age_budget_ms;
        }
        ObservationEvent::TransportMetrics {
            video_rtt_ms,
            video_rtt_source,
            inbound_video_loss_ratio_5s,
            inbound_video_loss_ratio_1s,
            transport_path,
            inbound_video_bitrate_kbps,
            inbound_primary_video_bytes_total,
        } => {
            stats.video_rtt_ms = video_rtt_ms;
            stats.video_rtt_source = video_rtt_source;
            stats.inbound_video_loss_ratio_5s = inbound_video_loss_ratio_5s;
            stats.inbound_video_loss_ratio_1s = inbound_video_loss_ratio_1s;
            stats.transport_path = transport_path;
            stats.inbound_primary_video_bytes_total = inbound_primary_video_bytes_total;
            stats.inbound_video_bytes_total = inbound_primary_video_bytes_total;
            stats.inbound_video_bitrate_kbps = Some(inbound_video_bitrate_kbps.max(0.0));
            stats.inbound_bitrate_kbps = Some(
                inbound_video_bitrate_kbps.max(0.0)
                    + stats.inbound_audio_bitrate_kbps.unwrap_or(0.0),
            );
            stats.inbound_bytes_total =
                stats.inbound_video_bytes_total + stats.inbound_audio_bytes_total;
        }
        ObservationEvent::VideoFrameDrop { observation } => {
            stats.latest_video_frame_drop = Some(observation);
        }
        ObservationEvent::InboundVideoPacketLossEstimate { packet_count } => {
            stats.inbound_video_packet_loss_estimate_total = stats
                .inbound_video_packet_loss_estimate_total
                .saturating_add(u64::from(packet_count));
        }
        ObservationEvent::VideoLossFinalized { packet_count } => {
            stats.video_loss_finalized_count_total = stats
                .video_loss_finalized_count_total
                .saturating_add(packet_count as u64);
        }
        ObservationEvent::VideoPendingMissingPackets { pending_count } => {
            stats.video_pending_missing_packets = pending_count;
        }
        ObservationEvent::NackSent {
            batch_len,
            pending_count,
        } => {
            stats.video_nack_batch_count_total =
                stats.video_nack_batch_count_total.saturating_add(1);
            stats.video_nack_request_count_total = stats
                .video_nack_request_count_total
                .saturating_add(batch_len as u64);
            stats.video_pending_missing_packets = pending_count;
        }
        ObservationEvent::LatestVideoNackObservation { observation } => {
            if let Some(active_since_ms) = stats.video_repair_probe_active_since_ms {
                if observation.observed_at_ms >= active_since_ms
                    && observation.action.starts_with("expired")
                {
                    stats.video_repair_probe_expired_count_since_active = stats
                        .video_repair_probe_expired_count_since_active
                        .saturating_add(1);
                }
            }
            stats.latest_video_nack_observation = Some(observation);
        }
        ObservationEvent::NackRecovered {
            was_late,
            recovery_time_ms,
            pending_count,
            observation,
        } => {
            stats.video_pending_missing_packets = pending_count;
            if was_late {
                stats.video_loss_late_recovered_count_total = stats
                    .video_loss_late_recovered_count_total
                    .saturating_add(1);
                if stats
                    .video_repair_probe_active_since_ms
                    .is_some_and(|active_since_ms| observation.observed_at_ms >= active_since_ms)
                {
                    stats.video_repair_probe_late_recovered_count_since_active = stats
                        .video_repair_probe_late_recovered_count_since_active
                        .saturating_add(1);
                }
            } else {
                stats.video_loss_recovered_count_total =
                    stats.video_loss_recovered_count_total.saturating_add(1);
                if stats
                    .video_repair_probe_active_since_ms
                    .is_some_and(|active_since_ms| observation.observed_at_ms >= active_since_ms)
                {
                    stats.video_repair_probe_recovered_count_since_active = stats
                        .video_repair_probe_recovered_count_since_active
                        .saturating_add(1);
                }
            }
            stats.video_nack_recovery_rtt_ms = Some(recovery_time_ms);
            stats.latest_video_nack_observation = Some(observation);
        }
        ObservationEvent::LatestVideoPacketGap {
            observation,
            latest_sequence,
        } => {
            if stats
                .video_repair_probe_active_since_ms
                .is_some_and(|active_since_ms| observation.observed_at_ms >= active_since_ms)
            {
                stats.video_repair_probe_packet_gap_count_since_active = stats
                    .video_repair_probe_packet_gap_count_since_active
                    .saturating_add(1);
            }
            stats.latest_video_packet_gap = Some(observation);
            stats.latest_video_packet_sequence = Some(latest_sequence);
        }
    }
}
