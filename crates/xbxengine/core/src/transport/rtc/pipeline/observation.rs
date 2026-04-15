use crate::{
    api::backend::XbxEngineVideoFrameDropObservation,
    media::video::ingress::scheduler::IngressDecision,
    media::video::types::FrameRecoveryDisposition, runtime_stats_sink::RuntimeStatsSink,
    transport::rtc::stream::adapter_types::TransportObservation,
};
use std::collections::VecDeque;

pub(super) struct MediaSupervisorObservationState {
    frame_count: u64,
    frame_drop_observation_id: u64,
    recent_receive_frame_times_ms: VecDeque<f64>,
    last_transport_escalation_hint: Option<(String, f64)>,
}

impl MediaSupervisorObservationState {
    pub(super) fn new() -> Self {
        Self {
            frame_count: 0,
            frame_drop_observation_id: 0,
            recent_receive_frame_times_ms: VecDeque::new(),
            last_transport_escalation_hint: None,
        }
    }

    pub(super) fn record_frame_arrival(
        &mut self,
        runtime_stats: &RuntimeStatsSink,
        now_ms: f64,
    ) -> u64 {
        self.frame_count = self.frame_count.saturating_add(1);
        self.recent_receive_frame_times_ms.push_back(now_ms);
        trim_recent_times(&mut self.recent_receive_frame_times_ms, now_ms);
        runtime_stats.record_frame_arrival(
            now_ms,
            self.frame_count,
            calculate_recent_fps(&self.recent_receive_frame_times_ms),
        );
        self.frame_count
    }

    pub(super) fn record_stream_dimensions(
        &self,
        runtime_stats: &RuntimeStatsSink,
        width: u32,
        height: u32,
    ) {
        runtime_stats.record_stream_dimensions(width, height);
    }

    pub(super) fn record_ingress_observation(
        &mut self,
        runtime_stats: &RuntimeStatsSink,
        decision: &IngressDecision,
        reason: Option<&str>,
        reconfigure_reason: Option<&str>,
        observed_at_ms: f64,
        width: u32,
        height: u32,
        is_keyframe: bool,
        queue_depth: usize,
        frame_rtp_timestamp: Option<u32>,
        frame_seq: Option<u64>,
        frame_recovery_disposition: Option<FrameRecoveryDisposition>,
        frame_unrecoverable_reason: Option<&str>,
    ) {
        if !matches!(
            decision,
            IngressDecision::DropLate
                | IngressDecision::DropBacklogIncoming
                | IngressDecision::DropBacklogEvictQueued
                | IngressDecision::DropUnrecoverable
                | IngressDecision::WaitKeyframe
                | IngressDecision::Reconfigure
        ) {
            return;
        }

        record_pipeline_frame_drop(
            runtime_stats,
            &mut self.frame_drop_observation_id,
            "ingress",
            map_ingress_action(decision),
            Some(map_ingress_drop_reason(decision, reason, reconfigure_reason).as_str()),
            observed_at_ms,
            width,
            height,
            is_keyframe,
            queue_depth,
            frame_rtp_timestamp,
            frame_seq,
            frame_recovery_disposition,
            frame_unrecoverable_reason,
        );
    }

    pub(super) fn total_frame_count(&self) -> u64 {
        self.frame_count
    }

    pub(super) fn should_log_transport_hint(&self, label: &str, now_ms: f64) -> bool {
        match self.last_transport_escalation_hint.as_ref() {
            Some((last_label, last_at_ms)) => {
                last_label != label || now_ms - *last_at_ms >= 1_000.0
            }
            None => true,
        }
    }

    pub(super) fn record_transport_hint(&mut self, label: String, now_ms: f64) {
        self.last_transport_escalation_hint = Some((label, now_ms));
    }
}

fn trim_recent_times(times: &mut VecDeque<f64>, now_ms: f64) {
    while let Some(front) = times.front().copied() {
        if now_ms - front <= 1_000.0 {
            break;
        }
        times.pop_front();
    }
}

fn calculate_recent_fps(times: &VecDeque<f64>) -> f64 {
    let len = times.len();
    if len < 2 {
        return 0.0;
    }
    let first = times.front().copied().unwrap_or_default();
    let last = times.back().copied().unwrap_or(first);
    let window_ms = (last - first).max(1.0);
    ((len.saturating_sub(1)) as f64 * 1_000.0 / window_ms).max(0.0)
}

fn map_ingress_drop_reason(
    decision: &IngressDecision,
    reason: Option<&str>,
    reconfigure_reason: Option<&str>,
) -> String {
    match decision {
        IngressDecision::Submit => "submit".to_string(),
        IngressDecision::DropLate => "dropLate".to_string(),
        IngressDecision::DropBacklogIncoming => "dropBacklogIncoming".to_string(),
        IngressDecision::DropBacklogEvictQueued => "dropBacklogEvictQueued".to_string(),
        IngressDecision::DropUnrecoverable => {
            let detail = reason.unwrap_or("late");
            let wait_keyframe_hint = if detail == "referenceChain" {
                ";waitKeyframeEntered:referenceChain"
            } else {
                ""
            };
            format!(
                "frameAbandoned:{detail};frameRecoveryDisposition=unrecoverable;recoveryStrategyMode=latency-first{wait_keyframe_hint}"
            )
        }
        IngressDecision::WaitKeyframe => {
            if let Some(detail) = reason {
                format!(
                    "waitKeyframeEntered:{detail};frameRecoveryDisposition=waitKeyframe;recoveryStrategyMode=latency-first"
                )
            } else {
                "waitKeyframe".to_string()
            }
        }
        IngressDecision::Reconfigure => {
            if let Some(reason) = reconfigure_reason {
                format!("reconfigure:{reason}")
            } else {
                "reconfigure".to_string()
            }
        }
    }
}

fn map_ingress_action(decision: &IngressDecision) -> &'static str {
    match decision {
        IngressDecision::Submit => "submit",
        IngressDecision::DropLate => "drop",
        IngressDecision::DropBacklogIncoming => "drop",
        IngressDecision::DropBacklogEvictQueued => "evict",
        IngressDecision::DropUnrecoverable => "drop",
        IngressDecision::WaitKeyframe => "defer",
        IngressDecision::Reconfigure => "reconfigure",
    }
}

pub(super) fn map_transport_observation_to_hint_label(
    observation: &TransportObservation,
    severe_deadline_packet_threshold: usize,
) -> &'static str {
    match observation {
        TransportObservation::Admission(
            crate::transport::rtc::stream::adapter_types::TransportAdmissionObservation::AwaitRecoveryKeyframe,
        ) => "transportAwaitRecoveryKeyframe",
        TransportObservation::Loss(
            crate::transport::rtc::stream::adapter_types::TransportLossObservation::PacketLossDetected,
        ) => "transportSampleLoss",
        TransportObservation::Loss(
            crate::transport::rtc::stream::adapter_types::TransportLossObservation::RecoveryKeyframeRequested,
        ) => "transportRecoveryKeyframeRequested",
        TransportObservation::Loss(
            crate::transport::rtc::stream::adapter_types::TransportLossObservation::AwaitRecoveryKeyframe,
        ) => "transportAwaitRecoveryKeyframe",
        TransportObservation::StreamIdleTimeout => "adapterIdleTimeout",
        TransportObservation::StreamThinStall => "adapterThinStream",
        TransportObservation::NackRecoveredLate => "transportRecoveredLate",
        TransportObservation::NackDeadlineExpired { missing_packets } => {
            if usize::from(*missing_packets) >= severe_deadline_packet_threshold {
                "transportSevereDeadline"
            } else {
                "transportExpiredDeadline"
            }
        }
    }
}

pub(super) fn transport_observation_severity(observation: &TransportObservation) -> u8 {
    match observation {
        TransportObservation::NackDeadlineExpired { missing_packets } if *missing_packets >= 64 => {
            2
        }
        TransportObservation::StreamIdleTimeout
        | TransportObservation::StreamThinStall
        | TransportObservation::NackDeadlineExpired { .. } => 1,
        TransportObservation::Admission(_)
        | TransportObservation::Loss(_)
        | TransportObservation::NackRecoveredLate => 0,
    }
}

pub(crate) fn record_pipeline_frame_drop(
    runtime_stats: &RuntimeStatsSink,
    observation_id: &mut u64,
    stage: &'static str,
    action: &'static str,
    detail: Option<&str>,
    observed_at_ms: f64,
    width: u32,
    height: u32,
    is_keyframe: bool,
    queue_depth: usize,
    frame_rtp_timestamp: Option<u32>,
    frame_seq: Option<u64>,
    frame_recovery_disposition: Option<FrameRecoveryDisposition>,
    frame_unrecoverable_reason: Option<&str>,
) {
    *observation_id = observation_id.saturating_add(1);
    let reason = match (stage, detail) {
        ("pacer", Some("deadline")) => "dropLate".to_string(),
        ("ingress", Some(detail)) => detail.to_string(),
        (_, Some(detail)) => format!("{stage}:{action}:{detail}"),
        _ => format!("{stage}:{action}"),
    };
    runtime_stats.record_video_frame_drop(XbxEngineVideoFrameDropObservation {
        observation_id: *observation_id,
        reason,
        stage: Some(stage.to_string()),
        action: Some(action.to_string()),
        detail: detail.map(str::to_string),
        frame_rtp_timestamp,
        frame_seq,
        frame_recovery_disposition: frame_recovery_disposition
            .map(FrameRecoveryDisposition::as_str)
            .map(str::to_string),
        frame_unrecoverable_reason: frame_unrecoverable_reason.map(str::to_string),
        frame_budget: None,
        observed_at_ms,
        width,
        height,
        is_keyframe,
        queue_depth,
    });
}

#[cfg(test)]
mod tests {
    use super::map_transport_observation_to_hint_label;
    use crate::transport::rtc::stream::adapter_types::{
        TransportLossObservation, TransportObservation,
    };

    #[test]
    fn recovery_keyframe_requested_maps_to_distinct_recovery_request_label() {
        let label = map_transport_observation_to_hint_label(
            &TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested),
            64,
        );
        assert_eq!(label, "transportRecoveryKeyframeRequested");
    }
}
